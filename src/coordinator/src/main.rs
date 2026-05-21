//! Pinaivu Coordinator — entry point.
//!
//! Generates a single `EnclaveKeyPair` and shares it across the libp2p
//! swarm (used to derive PeerId), the HTTP signing layer, and the
//! routing-receipt signer. Connects Postgres + Redis, spawns the
//! apalis deadline watcher alongside the libp2p mesh and the HTTP
//! server, then waits on `ctrl-c` for graceful shutdown.

use std::sync::Arc;
use std::time::Duration;

use anyhow::{Context, Result};
use base64::Engine;
use coordinator::{
    app,
    jobs::{store::PgJobStore, worker::spawn_dispatch_worker},
    mesh::{spawn_libp2p_mesh_full, PeerRegistry},
    observability,
    onchain::{spawn_registration, RegisteredEnclave, SidecarClient},
    persistence::{postgres as pg, redis as r},
    receipts::PostgresReceiptArchive,
    settlement::{free::FreeSettlement, SettlementAdapter},
};
use nautilus_enclave::EnclaveKeyPair;
use tokio::sync::RwLock;

/// How long the in-enclave peer registry holds entries between
/// gossip announcements before eviction.
const PEER_TTL: Duration = Duration::from_secs(600);

#[tokio::main]
async fn main() {
    match run().await {
        Ok(()) => {}
        Err(e) => {
            eprintln!("FATAL: coordinator exited with error: {e:?}");
            std::process::exit(1);
        }
    }
}

async fn run() -> Result<()> {
    eprintln!("CHK 01 main entered");
    observability::init();
    eprintln!("CHK 02 observability init");

    let cfg = app::Config::from_env()?;
    eprintln!(
        "CHK 03 config loaded: database_url_len={} redis_url_len={}",
        cfg.database_url.len(),
        cfg.redis_url.len()
    );

    let enclave_key = Arc::new(EnclaveKeyPair::generate());
    tracing::info!(
        coordinator_pubkey = %hex::encode(enclave_key.public_key_bytes()),
        "enclave key generated"
    );
    eprintln!("CHK 04 enclave key generated");

    eprintln!("CHK 05 connecting postgres…");
    let pg_pool = pg::connect(&cfg.database_url)
        .await
        .context("connect postgres")?;
    eprintln!("CHK 06 postgres connected");
    tracing::info!("postgres connected; migrations applied");

    eprintln!("CHK 07 connecting redis…");
    let _redis = r::connect(&cfg.redis_url).await.context("connect redis")?;
    eprintln!("CHK 08 redis connected");
    tracing::info!("redis connected (PONG)");

    // ── Apalis deadline-watcher worker ─────────────────────────────────────
    // Watches `dispatch_jobs` for deadline-elapsed entries with no
    // CompletionAck and fires the settlement refund path.
    let job_store = PgJobStore::new(pg_pool.clone())
        .await
        .context("init apalis job store")?;
    let settlement: Arc<dyn SettlementAdapter> = Arc::new(FreeSettlement);
    let _worker_handle =
        spawn_dispatch_worker(&job_store, pg_pool.clone(), settlement);
    tracing::info!("apalis dispatch-timeout worker spawned");

    // ── Settlement worker ──────────────────────────────────────────────────
    // Drains `pending` payment rows and submits vault::settle PTBs via
    // the in-enclave sidecar. Only spawned when the sidecar is reachable.
    // `settlement_tx` is passed into the mesh event loop so it can trigger
    // the worker immediately after inserting payment rows.
    let settlement_tx: Option<tokio::sync::mpsc::Sender<coordinator::jobs::settlement_worker::SettlementJob>> =
        match SidecarClient::from_env() {
            Ok(sidecar) => {
                use apalis::prelude::Storage;
                use apalis_sql::postgres::PostgresStorage;
                use coordinator::jobs::settlement_worker::{spawn_settlement_worker, SettlementJob};
                let storage = PostgresStorage::<SettlementJob>::new(pg_pool.clone());
                let handle = spawn_settlement_worker(pg_pool.clone(), Arc::new(sidecar), storage.clone());
                tracing::info!("apalis settlement worker spawned");
                let _worker_handle = handle;

                // Channel bridges the event loop (which can't hold apalis storage directly)
                // to a background task that pushes jobs into the apalis Postgres queue.
                let (tx, mut rx) = tokio::sync::mpsc::channel::<SettlementJob>(64);
                let mut push_storage = storage;
                tokio::spawn(async move {
                    while let Some(job) = rx.recv().await {
                        let req = apalis::prelude::Request::new(job);
                        if let Err(e) = push_storage.push_request(req).await {
                            tracing::error!(err = %e, "failed to enqueue settlement job");
                        }
                    }
                });
                Some(tx)
            }
            Err(_) => {
                tracing::warn!("sidecar unavailable — settlement worker not started");
                None
            }
        };

    // ── Receipt archive ────────────────────────────────────────────────────
    let receipt_archive: Arc<dyn coordinator::receipts::ReceiptArchive> =
        Arc::new(PostgresReceiptArchive::new(pg_pool.clone()));

    // ── libp2p mesh ────────────────────────────────────────────────────────
    let peer_registry = Arc::new(PeerRegistry::new(PEER_TTL));
    let listen_addr = cfg
        .libp2p_listen
        .parse()
        .map_err(|e| anyhow::anyhow!("PINAIVU_LIBP2P_LISTEN must be a multiaddr: {e}"))?;

    let mesh_handle = spawn_libp2p_mesh_full(
        enclave_key.clone(),
        listen_addr,
        peer_registry.clone(),
        receipt_archive.clone(),
        Some(pg_pool.clone()),
        settlement_tx,
    )
    .await?;
    for addr in &mesh_handle.listen_addrs {
        tracing::info!(libp2p_addr = %addr, "mesh listening");
    }

    // ── On-chain registration via the colocated sidecar ───────────────────
    // Asks the TS sidecar to register this enclave on Sui so any
    // receipts we sign will verify under pinaivu::enclave. Warning-on-
    // fail with background retry; inference doesn't depend on this
    // (payouts will fail to settle on-chain until registration lands).
    let on_chain_state = Arc::new(RwLock::new(None::<RegisteredEnclave>));
    match SidecarClient::from_env() {
        Ok(sidecar) => {
            let pubkey = enclave_key.public_key_bytes();
            match nautilus_enclave::get_attestation(&pubkey, &[]) {
                Ok(doc) => {
                    if let Ok(att_bytes) = hex::decode(&doc.raw_cbor_hex) {
                        let att_b64 = base64::engine::general_purpose::STANDARD
                            .encode(&att_bytes);
                        spawn_registration(sidecar, att_b64, on_chain_state.clone());
                    } else {
                        tracing::warn!("attestation raw_cbor_hex is not valid hex; skipping registration");
                    }
                }
                Err(e) => tracing::warn!(?e, "NSM attestation failed; skipping registration"),
            }
        }
        Err(e) => tracing::warn!(?e, "sidecar client unavailable; skipping registration"),
    }

    // ── HTTP server ────────────────────────────────────────────────────────
    let state = app::AppState::with_full_archive_and_chain(
        enclave_key,
        mesh_handle.mesh.clone(),
        peer_registry,
        receipt_archive,
        on_chain_state,
    );
    state.set_pg_pool(pg_pool.clone()).await;

    let (listener, local) = coordinator::bind(&cfg.bind_addr).await?;
    tracing::info!(listening = %local, "coordinator http ready");

    let router = coordinator::build_router(state);
    axum::serve(listener, router)
        .with_graceful_shutdown(shutdown_signal())
        .await?;

    tracing::info!("coordinator exited cleanly");
    Ok(())
}

async fn shutdown_signal() {
    if let Err(err) = tokio::signal::ctrl_c().await {
        tracing::warn!(?err, "failed to install ctrl_c handler");
    }
    tracing::info!("shutdown signal received");
}
