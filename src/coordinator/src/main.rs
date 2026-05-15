//! Pinaivu Coordinator — entry point.
//!
//! Loads config, generates the enclave keypair, binds the HTTP server,
//! and waits on `ctrl-c` for graceful shutdown. Mesh + apalis monitor
//! land in subsequent slices.

use anyhow::Result;
use coordinator::{app, build_router, observability};

#[tokio::main]
async fn main() -> Result<()> {
    observability::init();

    let _cfg = app::Config::from_env()?;
    let state = app::AppState::new();

    tracing::info!(
        coordinator_pubkey = %hex::encode(state.enclave_pubkey_bytes()),
        "enclave key generated"
    );

    let bind_addr = std::env::var("PINAIVU_BIND").unwrap_or_else(|_| "127.0.0.1:4000".into());
    let (listener, local) = coordinator::bind(&bind_addr).await?;
    tracing::info!(listening = %local, "coordinator http ready");

    let router = build_router(state);
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
