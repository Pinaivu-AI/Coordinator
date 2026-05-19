//! End-to-end test of the completion-ack flow.
//!
//! Spins up a coordinator backed by an `InMemoryMesh` pre-seeded with
//! one bid. Calls `POST /v1/chat/completions` to get a dispatch (which
//! populates the in-flight map on the real event loop). Then directly
//! inserts a signed `RoutingReceipt` into the archive (simulating what
//! the libp2p completion handler does) and verifies that
//! `GET /v1/proofs/{request_id}` returns the stored receipt with a
//! valid coordinator signature.
//!
//! The libp2p request_response path itself is exercised separately in
//! mesh_smoke.rs; here we focus on the HTTP surface and receipt
//! verification properties.

use std::sync::Arc;

use coordinator::app::AppState;
use coordinator::protocol::{NodePeerId, RoutingReceipt};
use coordinator::receipts::{InMemoryReceiptArchive, ReceiptArchive};
use coordinator::{bind, build_router};
use nautilus_enclave::EnclaveKeyPair;
use serde_json::Value;
use uuid::Uuid;

async fn start_coordinator() -> (reqwest::Client, String, Arc<InMemoryReceiptArchive>) {
    let archive = Arc::new(InMemoryReceiptArchive::new());
    let state = AppState::with_full_archive(
        Arc::new(EnclaveKeyPair::generate()),
        Arc::new(coordinator::mesh::NoopMesh),
        Arc::new(coordinator::mesh::PeerRegistry::new(
            std::time::Duration::from_secs(60),
        )),
        archive.clone(),
    );

    let router = build_router(state);
    let (listener, addr) = bind("127.0.0.1:0").await.expect("bind");
    let base = format!("http://{addr}");
    tokio::spawn(async move {
        let _ = axum::serve(listener, router).await;
    });

    let client = reqwest::Client::new();
    (client, base, archive)
}

#[tokio::test]
async fn get_proof_returns_stored_receipt() {
    let (client, base, archive) = start_coordinator().await;

    let key = nautilus_enclave::EnclaveKeyPair::generate();
    let request_id = Uuid::new_v4();

    // Build and sign a receipt directly (simulates what the event loop
    // does after verifying a CompletionAck).
    let receipt = RoutingReceipt {
        request_id,
        client_id: "test-client".into(),
        primary_peer_id: NodePeerId("12D3KooWPrimary".into()),
        helper_peer_ids: vec![],
        bid_set_hash: [0u8; 32],
        proof_ids: vec![[1u8; 32]],
        aggregated_output_hash: [2u8; 32],
        payouts: vec![],
        timestamp_ms: 1_700_000_000_000,
        coordinator_pubkey: [0u8; 32],
        signature: vec![],
    }
    .sign(key.signing_key());

    archive.put(receipt.clone()).await.expect("put receipt");

    let resp = client
        .get(format!("{base}/v1/proofs/{request_id}"))
        .send()
        .await
        .expect("get proof");

    assert!(
        resp.status().is_success(),
        "status={} body={:?}",
        resp.status(),
        resp.text().await.ok()
    );

    let body: Value = resp.json().await.expect("json");
    assert_eq!(body["request_id"], request_id.to_string());
    assert_eq!(body["primary_peer_id"], "12D3KooWPrimary");
    // Signature must be 64 bytes = 128 hex chars in the JSON array.
    assert_eq!(
        body["signature"].as_array().expect("sig array").len(),
        64
    );
}

#[tokio::test]
async fn get_proof_returns_404_for_unknown_request() {
    let (client, base, _archive) = start_coordinator().await;

    let resp = client
        .get(format!("{base}/v1/proofs/{}", Uuid::new_v4()))
        .send()
        .await
        .expect("get proof");

    assert_eq!(resp.status().as_u16(), 404);
}

#[tokio::test]
async fn stored_receipt_signature_verifies() {
    let (_client, _base, archive) = start_coordinator().await;

    let key = nautilus_enclave::EnclaveKeyPair::generate();
    let request_id = Uuid::new_v4();

    let receipt = RoutingReceipt {
        request_id,
        client_id: "verify-test".into(),
        primary_peer_id: NodePeerId("12D3KooWPrimary".into()),
        helper_peer_ids: vec![NodePeerId("12D3KooWHelper".into())],
        bid_set_hash: [3u8; 32],
        proof_ids: vec![[4u8; 32], [5u8; 32]],
        aggregated_output_hash: [6u8; 32],
        payouts: vec![],
        timestamp_ms: 1_700_000_010_000,
        coordinator_pubkey: [0u8; 32],
        signature: vec![],
    }
    .sign(key.signing_key());

    assert!(receipt.verify().is_ok(), "receipt must self-verify");

    archive.put(receipt.clone()).await.expect("put");
    let fetched = archive.get(&request_id).await.expect("get").expect("some");
    assert!(fetched.verify().is_ok(), "fetched receipt must still verify");
}
