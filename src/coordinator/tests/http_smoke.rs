//! Integration smoke test for the coordinator's HTTP surface.
//!
//! Stands the coordinator up on an ephemeral port, hits `/health`,
//! `/enclave_health`, and `/get_attestation`, and verifies the
//! returned attestation document carries the same pubkey the enclave
//! exposes via `/enclave_health`.

use coordinator::{app::AppState, bind, build_router_no_auth as build_router};
use serde::Deserialize;

#[derive(Deserialize)]
struct EnclaveHealth {
    public_key_hex: String,
    uptime_ms: u64,
}

#[derive(Deserialize)]
struct AttestationDoc {
    pcr0: String,
    pcr1: String,
    pcr2: String,
    public_key: String,
    #[allow(dead_code)]
    timestamp_ms: u64,
    #[allow(dead_code)]
    raw_cbor_hex: String,
}

async fn spawn_coordinator() -> (String, tokio::task::JoinHandle<()>) {
    let state = AppState::new();
    let router = build_router(state);
    let (listener, addr) = bind("127.0.0.1:0").await.expect("bind ephemeral");
    let base = format!("http://{addr}");
    let handle = tokio::spawn(async move {
        let _ = axum::serve(listener, router).await;
    });
    // Give the server a moment to start accepting connections.
    tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    (base, handle)
}

#[tokio::test]
async fn health_endpoint_returns_ok() {
    let (base, handle) = spawn_coordinator().await;
    let body = reqwest::get(format!("{base}/health"))
        .await
        .expect("request")
        .text()
        .await
        .expect("text");
    assert_eq!(body, "ok");
    handle.abort();
}

#[tokio::test]
async fn enclave_health_returns_pubkey_and_uptime() {
    let (base, handle) = spawn_coordinator().await;
    let resp: EnclaveHealth = reqwest::get(format!("{base}/enclave_health"))
        .await
        .expect("request")
        .json()
        .await
        .expect("json");
    // 32 bytes hex = 64 chars
    assert_eq!(resp.public_key_hex.len(), 64);
    assert!(
        resp.public_key_hex.chars().all(|c| c.is_ascii_hexdigit()),
        "pubkey must be valid hex: {}",
        resp.public_key_hex
    );
    // Uptime should be small but non-negative; just ensure the field
    // parsed correctly.
    let _ = resp.uptime_ms;
    handle.abort();
}

#[tokio::test]
async fn attestation_pubkey_matches_enclave_health_pubkey() {
    let (base, handle) = spawn_coordinator().await;

    let health: EnclaveHealth = reqwest::get(format!("{base}/enclave_health"))
        .await
        .expect("request")
        .json()
        .await
        .expect("json");

    let doc: AttestationDoc = reqwest::get(format!("{base}/get_attestation"))
        .await
        .expect("request")
        .json()
        .await
        .expect("json");

    // The attestation must commit to the same pubkey the health
    // endpoint exposes — otherwise a client cannot use the attested
    // pubkey to verify subsequent coordinator-signed artefacts.
    assert_eq!(doc.public_key, health.public_key_hex);

    // PCRs are SHA-shaped: 48 bytes hex = 96 chars in our mock impl.
    assert_eq!(doc.pcr0.len(), 96);
    assert_eq!(doc.pcr1.len(), 96);
    assert_eq!(doc.pcr2.len(), 96);

    handle.abort();
}
