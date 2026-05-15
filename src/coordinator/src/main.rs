//! Pinaivu Coordinator — entry point.
//!
//! Runs inside an AWS Nitro Enclave. Control-plane component for the
//! Pinaivu decentralised AI inference marketplace: runs the libp2p
//! auction on behalf of the client, signs a dispatch token, tracks
//! the job via apalis, verifies the completion ack from `node_1`,
//! and issues a routing receipt.
//!
//! This is the foundation scaffold. Marketplace / mesh / settlement
//! modules land in subsequent commits.

mod attestation;
mod config;
mod error;
mod identity;
mod state;
mod telemetry;

use anyhow::Result;

#[tokio::main]
async fn main() -> Result<()> {
    telemetry::init();
    tracing::info!("pinaivu coordinator starting (scaffold)");
    let _cfg = config::Config::from_env()?;
    let _state = state::AppState::new();
    // TODO(scaffold): identity -> mesh -> http -> apalis monitor -> shutdown.
    Ok(())
}
