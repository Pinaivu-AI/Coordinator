//! Pinaivu Coordinator — entry point.
//!
//! Generates a single `EnclaveKeyPair` and shares it across the libp2p
//! swarm (used to derive PeerId), the HTTP signing layer, and the
//! routing-receipt signer. Spawns the libp2p mesh task, binds the HTTP
//! server, waits on `ctrl-c` for graceful shutdown.

use std::sync::Arc;
use std::time::Duration;

use anyhow::Result;
use coordinator::{
    app,
    mesh::{spawn_libp2p_mesh, PeerRegistry},
    observability,
    receipts::InMemoryReceiptArchive,
};
use nautilus_enclave::EnclaveKeyPair;

/// How long the in-enclave peer registry holds entries between
/// gossip announcements before eviction.
const PEER_TTL: Duration = Duration::from_secs(600);

#[tokio::main]
async fn main() -> Result<()> {
    observability::init();

    let _cfg = app::Config::from_env()?;

    // Single source of truth for the coordinator's cryptographic
    // identity. Same key seeds the libp2p PeerId, signs HTTP
    // responses, and (slice 5.4) signs routing receipts.
    let enclave_key = Arc::new(EnclaveKeyPair::generate());
    tracing::info!(
        coordinator_pubkey = %hex::encode(enclave_key.public_key_bytes()),
        "enclave key generated"
    );

    let peer_registry = Arc::new(PeerRegistry::new(PEER_TTL));

    let listen_str = std::env::var("PINAIVU_LIBP2P_LISTEN")
        .unwrap_or_else(|_| "/ip4/0.0.0.0/tcp/0".into());
    let listen_addr = listen_str
        .parse()
        .map_err(|e| anyhow::anyhow!("PINAIVU_LIBP2P_LISTEN must be a multiaddr: {e}"))?;

    let receipt_archive = Arc::new(InMemoryReceiptArchive::new());

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

    let state = app::AppState::with_full_archive(
        enclave_key,
        mesh_handle.mesh.clone(),
        peer_registry,
        receipt_archive,
    );

    let bind_addr = std::env::var("PINAIVU_BIND").unwrap_or_else(|_| "127.0.0.1:4000".into());
    let (listener, local) = coordinator::bind(&bind_addr).await?;
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
