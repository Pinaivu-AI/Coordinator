//! Pinaivu Coordinator — entry point.
//!
//! Generates a single `EnclaveKeyPair` and shares it across the libp2p
//! swarm (used to derive PeerId), the HTTP signing key, and the
//! routing-receipt signer. Connects Postgres + Redis, spawns the
//! apalis deadline watcher alongside the libp2p mesh and the HTTPS
//! server, then waits on `ctrl-c` for graceful shutdown.

use std::sync::Arc;
use std::time::Duration;

use anyhow::{Context, Result};
use base64::Engine;
use coordinator::{
    app,
    generate_self_signed_tls,
    jobs::{store::PgJobStore, worker::spawn_dispatch_worker},
    make_tls_config,
    mesh::{spawn_libp2p_mesh_full, PeerRegistry},
    observability,
    onchain::{spawn_registration, RegisteredEnclave, SidecarClient},
    persistence::{postgres as pg, redis as r},
    receipts::PostgresReceiptArchive,
    settlement::{free::FreeSettlement, SettlementAdapter},
};
use nautilus_enclave::EnclaveKeyPair;
use sha2::Digest;
use tokio::sync::RwLock;

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
        "CHK 03 config loaded: bind={} database_url_len={} redis_url_len={}",
        cfg.bind_addr,
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
    let redis_conn = r::connect(&cfg.redis_url).await.context("connect redis")?;
    eprintln!("CHK 08 redis connected");
    tracing::info!("redis connected (PONG)");

    // ── Apalis deadline-watcher worker ─────────────────────────────────────
    let job_store = PgJobStore::new(pg_pool.clone())
        .await
        .context("init apalis job store")?;
    let settlement: Arc<dyn SettlementAdapter> = Arc::new(FreeSettlement);
    let _worker_handle = spawn_dispatch_worker(&job_store, pg_pool.clone(), settlement);
    tracing::info!("apalis dispatch-timeout worker spawned");

    // ── Settlement worker ──────────────────────────────────────────────────
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

    // ── On-chain registration ──────────────────────────────────────────────
    let on_chain_state = Arc::new(RwLock::new(None::<RegisteredEnclave>));
    match SidecarClient::from_env() {
        Ok(sidecar) => {
            let pubkey = enclave_key.public_key_bytes();
            match nautilus_enclave::get_attestation(&pubkey, &[]) {
                Ok(doc) => {
                    if let Ok(att_bytes) = hex::decode(&doc.raw_cbor_hex) {
                        let att_b64 = base64::engine::general_purpose::STANDARD.encode(&att_bytes);
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

    // ── TLS setup ──────────────────────────────────────────────────────────
    // rustls 0.23 requires an explicit crypto provider. Install ring once
    // before any TLS operations; safe to call multiple times.
    rustls::crypto::ring::default_provider().install_default().ok();

    let san_ips: Vec<String> = std::env::var("PINAIVU_TLS_SAN_IPS")
        .unwrap_or_default()
        .split(',')
        .filter(|s| !s.is_empty())
        .map(str::to_string)
        .collect();

    let (tls_config, tls_fingerprint) = if cfg.has_tls_certs() {
        let cert_pem = cfg.tls_cert_pem.unwrap().into_bytes();
        let key_pem  = cfg.tls_key_pem.unwrap().into_bytes();
        let fp = cert_fingerprint_from_pem(&cert_pem).unwrap_or_else(|| "unknown".into());
        eprintln!("CHK 09 using operator TLS cert; fingerprint={fp}");
        (make_tls_config(cert_pem, key_pem).await?, fp)
    } else {
        eprintln!("CHK 09 generating self-signed TLS cert");
        generate_self_signed_tls(&san_ips).await?
    };
    eprintln!("CHK 10 TLS ready; fingerprint={tls_fingerprint}");

    // ── Build AppState ─────────────────────────────────────────────────────
    let state = app::AppState::with_full_archive_and_chain(
        enclave_key,
        mesh_handle.mesh.clone(),
        peer_registry,
        receipt_archive,
        on_chain_state,
    );
    state.set_pg_pool(pg_pool.clone()).await;
    state.set_redis(redis_conn).await;
    state.set_tls_cert_fingerprint(tls_fingerprint).await;

    // ── HTTPS server ───────────────────────────────────────────────────────
    let bind_addr: std::net::SocketAddr = cfg.bind_addr.parse()
        .map_err(|e| anyhow::anyhow!("invalid PINAIVU_BIND: {e}"))?;

    tracing::info!(listening = %bind_addr, "coordinator https ready");

    let handle = axum_server::Handle::new();
    let shutdown_handle = handle.clone();
    tokio::spawn(async move {
        shutdown_signal().await;
        shutdown_handle.graceful_shutdown(Some(Duration::from_secs(30)));
    });

    let router = coordinator::build_router(state);
    axum_server::bind_rustls(bind_addr, tls_config)
        .handle(handle)
        .serve(router.into_make_service())
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

/// SHA-256 fingerprint of the first DER cert in a PEM block.
fn cert_fingerprint_from_pem(pem: &[u8]) -> Option<String> {
    let text = std::str::from_utf8(pem).ok()?;
    let b64: String = text
        .lines()
        .skip_while(|l| !l.starts_with("-----BEGIN CERTIFICATE-----"))
        .skip(1)
        .take_while(|l| !l.starts_with("-----END CERTIFICATE-----"))
        .collect();
    let der = base64::engine::general_purpose::STANDARD.decode(b64).ok()?;
    Some(hex::encode(sha2::Sha256::digest(&der)))
}
