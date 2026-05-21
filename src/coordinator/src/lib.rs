//! Pinaivu coordinator — library surface.
//!
//! Exposes the module tree to both `main.rs` and integration tests.
//! The HTTP router and listener are constructed via [`build_router`]
//! and [`bind`] so tests can stand the service up on an ephemeral port.

pub mod api;
pub mod app;
pub mod jobs;
pub mod payments;
pub mod marketplace;
pub mod mesh;
pub mod observability;
pub mod onchain;
pub mod persistence;
pub mod receipts;
pub mod reputation;
pub mod settlement;

// Wire-format types live in the shared `pinaivu-protocol` crate so the
// node binary can reuse them. Re-exported here under `coordinator::protocol`
// so existing call sites and tests keep compiling unchanged.
pub use pinaivu_protocol as protocol;

use anyhow::Result;
use axum::{
    routing::{get, post},
    Router,
};
use tokio::net::TcpListener;

/// Build the axum router with the given shared state.
pub fn build_router(state: app::AppState) -> Router {
    Router::new()
        .route("/health", get(api::health::health))
        .route("/enclave_health", get(api::health::enclave_health))
        .route("/get_attestation", get(api::health::get_attestation))
        .route(
            "/v1/chat/completions",
            post(api::inference::chat_completions),
        )
        .route("/v1/nodes", get(api::nodes::list_nodes))
        .route("/v1/proofs/{request_id}", get(api::proofs::get_proof))
        .route("/v1/admin/set-enclave-id", post(api::admin::set_enclave_id))
        .with_state(state)
}

/// Bind a TCP listener and report the address it picked up. Pass
/// `"127.0.0.1:0"` to get an ephemeral port (used in tests).
pub async fn bind(addr: &str) -> Result<(TcpListener, std::net::SocketAddr)> {
    let listener = TcpListener::bind(addr).await?;
    let local = listener.local_addr()?;
    Ok((listener, local))
}
