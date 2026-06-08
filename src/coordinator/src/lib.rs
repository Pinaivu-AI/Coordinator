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
use axum_server::tls_rustls::RustlsConfig;
use sha2::{Digest, Sha256};
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
        .route(
            "/v1/admin/settlements/{request_id}",
            get(api::admin::settlement_status),
        )
        .route(
            "/v1/admin/sessions/{session_id}",
            get(api::admin::session_status),
        )
        .with_state(state)
}

/// Bind a TCP listener and report the address it picked up. Pass
/// `"127.0.0.1:0"` to get an ephemeral port (used in tests).
pub async fn bind(addr: &str) -> Result<(TcpListener, std::net::SocketAddr)> {
    let listener = TcpListener::bind(addr).await?;
    let local = listener.local_addr()?;
    Ok((listener, local))
}

/// Build a `RustlsConfig` from raw PEM bytes (cert + key).
/// Used when the operator supplies a real certificate via env vars.
pub async fn make_tls_config(cert_pem: Vec<u8>, key_pem: Vec<u8>) -> Result<RustlsConfig> {
    RustlsConfig::from_pem(cert_pem, key_pem)
        .await
        .map_err(|e| anyhow::anyhow!("TLS config from PEM: {e}"))
}

/// Generate a self-signed TLS certificate valid for `localhost` and
/// `127.0.0.1` using a fresh Ed25519 key. Returns `(RustlsConfig,
/// fingerprint)` where `fingerprint` is the SHA-256 hex digest of
/// the certificate's DER encoding — expose this via `/enclave_health`
/// so clients can pin the cert without trusting a CA.
///
/// Called at enclave boot when no operator cert is configured.
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
        .map_err(|e| anyhow::anyhow!("RustlsConfig from generated cert: {e}"))?;

    Ok((tls, fingerprint))
}
