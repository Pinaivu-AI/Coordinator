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
use coordinator::{
    app,
    jobs::{store::PgJobStore, worker::spawn_dispatch_worker},
    mesh::{spawn_libp2p_mesh, PeerRegistry},
    observability,
    persistence::{postgres as pg, redis as r},
    receipts::PostgresReceiptArchive,
    settlement::{free::FreeSettlement, SettlementAdapter},
};
use nautilus_enclave::EnclaveKeyPair;

/// How long the in-enclave peer registry holds entries between
/// gossip announcements before eviction.
const PEER_TTL: Duration = Duration::from_secs(600);

#[tokio::main]
async fn main() -> Result<()> {
    observability::init();

    let cfg = app::Config::from_env()?;

    // Single source of truth for the coordinator's cryptographic
    // identity. Same key seeds the libp2p PeerId, signs HTTP
    // responses, and signs routing receipts + dispatch tokens.
    let enclave_key = Arc::new(EnclaveKeyPair::generate());
    tracing::info!(
        coordinator_pubkey = %hex::encode(enclave_key.public_key_bytes()),
        "enclave key generated"
    );

    // ── Postgres: receipts + dispatch jobs + (later) payments ──────────────
    let pg_pool = pg::connect(&cfg.database_url)
        .await
        .context("connect postgres")?;
    tracing::info!("postgres connected; migrations applied");

    // ── Redis: replay nonces + hot caches ──────────────────────────────────
    let _redis = r::connect(&cfg.redis_url).await.context("connect redis")?;
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

    // ── Receipt archive ────────────────────────────────────────────────────
    let receipt_archive: Arc<dyn coordinator::receipts::ReceiptArchive> =
        Arc::new(PostgresReceiptArchive::new(pg_pool.clone()));

    // ── libp2p mesh ────────────────────────────────────────────────────────
    let peer_registry = Arc::new(PeerRegistry::new(PEER_TTL));
    let listen_addr = cfg
        .libp2p_listen
        .parse()
        .map_err(|e| anyhow::anyhow!("PINAIVU_LIBP2P_LISTEN must be a multiaddr: {e}"))?;

    let mesh_handle = spawn_libp2p_mesh(
        enclave_key.clone(),
        listen_addr,
        peer_registry.clone(),
        receipt_archive.clone(),
    )
    .await?;
    for addr in &mesh_handle.listen_addrs {
        tracing::info!(libp2p_addr = %addr, "mesh listening");
    }

    // ── HTTP server ────────────────────────────────────────────────────────
    let state = app::AppState::with_full_archive(
        enclave_key,
        mesh_handle.mesh.clone(),
        peer_registry,
        receipt_archive,
    );

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
