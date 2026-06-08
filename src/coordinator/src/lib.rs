//! Pinaivu coordinator — library surface.
//!
//! Exposes the module tree to both `main.rs` and integration tests.
//! The HTTP router and listener are constructed via [`build_router`]
//! and [`bind`] so tests can stand the service up on an ephemeral port.

pub mod api;
pub mod app;
pub mod auth;
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
    middleware,
    routing::{delete, get, post},
    Router,
};
use axum_server::tls_rustls::RustlsConfig;
use sha2::{Digest, Sha256};
use tokio::net::TcpListener;

/// Build the axum router for integration tests — no API key middleware.
/// Production always uses `build_router`.
/// Build the axum router without API key auth middleware.
/// For integration tests — production always uses `build_router`.
pub fn build_router_no_auth(state: app::AppState) -> Router {
    Router::new()
        .route("/health",          get(api::health::health))
        .route("/enclave_health",  get(api::health::enclave_health))
        .route("/get_attestation", get(api::health::get_attestation))
        .route("/v1/models",       get(api::models::list_models))
        .route("/v1/chat/completions",    post(api::inference::chat_completions))
        .route("/v1/nodes",               get(api::nodes::list_nodes))
        .route("/v1/proofs/{request_id}", get(api::proofs::get_proof))
        .route("/v1/admin/set-enclave-id",           post(api::admin::set_enclave_id))
        .route("/v1/admin/settlements/{request_id}", get(api::admin::settlement_status))
        .with_state(state)
}

/// Build the axum router.
///
/// Route layout:
///   Public (no auth)  — health, enclave_health, get_attestation, GET /v1/models
///   Protected (API key required) — POST /v1/chat/completions, GET /v1/nodes,
///                                   GET /v1/proofs/:id
///   Admin (x-sidecar-secret)    — /v1/admin/*, /v1/keys, /v1/accounts
pub fn build_router(state: app::AppState) -> Router {
    // ── Public routes ────────────────────────────────────────────────────────
    let public = Router::new()
        .route("/health",          get(api::health::health))
        .route("/enclave_health",  get(api::health::enclave_health))
        .route("/get_attestation", get(api::health::get_attestation))
        .route("/v1/models",       get(api::models::list_models));

    // ── API-key-protected routes ─────────────────────────────────────────────
    let protected = Router::new()
        .route("/v1/chat/completions",    post(api::inference::chat_completions))
        .route("/v1/nodes",               get(api::nodes::list_nodes))
        .route("/v1/proofs/{request_id}", get(api::proofs::get_proof))
        .route("/v1/usage",               get(api::usage::get_usage))
        .route_layer(middleware::from_fn_with_state(
            state.clone(),
            auth::require_api_key,
        ));

    // ── Admin / key-management routes (x-sidecar-secret) ────────────────────
    let admin = Router::new()
        .route("/v1/admin/set-enclave-id",           post(api::admin::set_enclave_id))
        .route("/v1/admin/settlements/{request_id}", get(api::admin::settlement_status))
        .route("/v1/accounts",                       post(api::keys::create_account))
        .route("/v1/keys",                           post(api::keys::create_key))
        .route("/v1/keys",                           get(api::keys::list_keys))
        .route("/v1/keys/{id}",                      delete(api::keys::revoke_key));

    Router::new()
        .merge(public)
        .merge(protected)
        .merge(admin)
        .with_state(state)
}

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
/// is the SHA-256 hex of the cert DER — expose via `/enclave_health`
/// so clients can pin without trusting a CA.
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
