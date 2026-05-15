//! Pinaivu Coordinator — entry point.
//!
//! Runs inside an AWS Nitro Enclave. Control-plane component for the
//! Pinaivu decentralised AI inference marketplace: runs the libp2p
//! auction on behalf of the client, signs a dispatch token, tracks the
//! job via apalis, verifies the completion ack from the primary node,
//! and issues a signed routing receipt.

mod api;
mod app;
mod jobs;
mod marketplace;
mod mesh;
mod observability;
mod persistence;
mod protocol;
mod reputation;
mod settlement;

use anyhow::Result;

#[tokio::main]
async fn main() -> Result<()> {
    observability::init();
    tracing::info!("pinaivu coordinator starting (scaffold)");
    let _cfg = app::Config::from_env()?;
    let _state = app::AppState::new();
    // TODO(scaffold): enclave identity -> mesh swarm -> http server ->
    // apalis monitor -> graceful shutdown.
    Ok(())
}
