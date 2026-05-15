//! Pinaivu Coordinator — entry point.
//!
//! Spawns the libp2p mesh task, generates the enclave keypair, binds
//! the HTTP server, and waits on `ctrl-c` for graceful shutdown.

use std::sync::Arc;
use std::time::Duration;

use anyhow::Result;
use coordinator::{
    app,
    mesh::{spawn_libp2p_mesh, PeerRegistry},
    observability,
};

/// How long the in-enclave peer registry holds entries between
/// gossip announcements before eviction.
const PEER_TTL: Duration = Duration::from_secs(600);

#[tokio::main]
async fn main() -> Result<()> {
    observability::init();

    let _cfg = app::Config::from_env()?;

    // Generate the enclave keypair first so its secret seeds the
    // libp2p identity — coordinator's network PeerId = the same key
    // that's bound into its NSM attestation.
    let bootstrap_key = nautilus_enclave::EnclaveKeyPair::generate();
    let enclave_secret = bootstrap_key.secret_bytes();
    tracing::info!(
        coordinator_pubkey = %hex::encode(bootstrap_key.public_key_bytes()),
        "enclave key generated"
    );

    let peer_registry = Arc::new(PeerRegistry::new(PEER_TTL));

    // libp2p listen — default is ephemeral on loopback so dev runs
    // don't conflict with other services. Override via env in prod.
    let listen_str = std::env::var("PINAIVU_LIBP2P_LISTEN")
        .unwrap_or_else(|_| "/ip4/0.0.0.0/tcp/0".into());
    let listen_addr = listen_str
        .parse()
        .map_err(|e| anyhow::anyhow!("PINAIVU_LIBP2P_LISTEN must be a multiaddr: {e}"))?;

    let mesh_handle = spawn_libp2p_mesh(enclave_secret, listen_addr, peer_registry.clone()).await?;
    for addr in &mesh_handle.listen_addrs {
        tracing::info!(libp2p_addr = %addr, "mesh listening");
    }

    // Build AppState carrying the same mesh + registry the event loop
    // is driving. Note: AppState generates its OWN enclave key —
    // we'll unify these in a follow-up so the HTTP-signing key and
    // the libp2p-identity key are the same EnclaveKeyPair instance.
    let _ = bootstrap_key; // keep alive until state owns it
    let state = app::AppState::with_mesh_and_registry(mesh_handle.mesh.clone(), peer_registry);

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
