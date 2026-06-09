//! Pinaivu coordinator — library surface.
//!
//! Exposes the module tree to both `main.rs` and integration tests.
//! The HTTP router and listener are constructed via [`build_router`]
//! and [`bind`] so tests can stand the service up on an ephemeral port.
//!
//! API key management, rate limiting, and usage tracking live in the
//! separate `pinaivu-api` repo (gateway + dashboard).

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

pub use pinaivu_protocol as protocol;

use anyhow::Result;
use axum::{
    routing::{get, post},
    Router,
};
use axum_server::tls_rustls::RustlsConfig;
use sha2::{Digest, Sha256};
use tokio::net::TcpListener;

/// Build the axum router. Auth is handled upstream by the API gateway;
/// the coordinator trusts all incoming requests.
pub fn build_router(state: app::AppState) -> Router {
    Router::new()
        // ── Liveness + attestation ─────────────────────────────────────────
        .route("/health",          get(api::health::health))
        .route("/enclave_health",  get(api::health::enclave_health))
        .route("/get_attestation", get(api::health::get_attestation))
        // ── Inference + peer discovery ─────────────────────────────────────
        .route("/v1/chat/completions",    post(api::inference::chat_completions))
        .route("/v1/models",              get(api::models::list_models))
        .route("/v1/nodes",               get(api::nodes::list_nodes))
        .route("/v1/proofs/{request_id}", get(api::proofs::get_proof))
        // ── Admin ──────────────────────────────────────────────────────────
        .route("/v1/admin/set-enclave-id",           post(api::admin::set_enclave_id))
        .route("/v1/admin/settlements/{request_id}", get(api::admin::settlement_status))
        .route("/v1/admin/sessions/{session_id}",    get(api::admin::session_status))
        .with_state(state)
}

/// Alias used by integration tests (same router, no auth middleware to strip).
pub use build_router as build_router_no_auth;

/// Bind a plain TCP listener. Used by integration tests (no TLS needed).
pub async fn bind(addr: &str) -> Result<(TcpListener, std::net::SocketAddr)> {
    let listener = TcpListener::bind(addr).await?;
    let local = listener.local_addr()?;
    Ok((listener, local))
}

/// Build a `RustlsConfig` from operator-supplied PEM bytes.
pub async fn make_tls_config(cert_pem: Vec<u8>, key_pem: Vec<u8>) -> Result<RustlsConfig> {
    RustlsConfig::from_pem(cert_pem, key_pem)
        .await
        .map_err(|e: std::io::Error| anyhow::anyhow!("TLS config from PEM: {e}"))
}

/// Generate a self-signed TLS certificate valid for `localhost` plus any
/// extra IP SANs. Returns `(RustlsConfig, fingerprint)` where `fingerprint`
/// is the SHA-256 hex of the cert DER.
pub async fn generate_self_signed_tls(san_ips: &[String]) -> Result<(RustlsConfig, String)> {
    use rcgen::{CertifiedKey, generate_simple_self_signed};

    let mut sans: Vec<String> = vec!["localhost".into()];
    sans.extend_from_slice(san_ips);

    let CertifiedKey { cert, key_pair } =
        generate_simple_self_signed(sans).map_err(|e| anyhow::anyhow!("rcgen: {e}"))?;

    let cert_pem = cert.pem().into_bytes();
    let key_pem  = key_pair.serialize_pem().into_bytes();
    let cert_der = cert.der().to_vec();
    let fingerprint = hex::encode(Sha256::digest(&cert_der));

    let tls = RustlsConfig::from_pem(cert_pem, key_pem)
        .await
        .map_err(|e: std::io::Error| anyhow::anyhow!("RustlsConfig from generated cert: {e}"))?;

    Ok((tls, fingerprint))
}
