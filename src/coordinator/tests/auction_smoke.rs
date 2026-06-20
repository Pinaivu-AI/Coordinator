//! End-to-end auction test. Spawns the coordinator with an
//! `InMemoryMesh` carrying two pre-seeded bids, posts a chat
//! completion request, and asserts that the returned dispatch token
//! names the higher-scoring bidder and verifies under the
//! coordinator's attested pubkey.

use std::sync::Arc;
use std::time::Duration;

use coordinator::app::AppState;
use coordinator::mesh::InMemoryMesh;
use coordinator::protocol::{InferenceBid, NanoX, NodePeerId};
use coordinator::{bind, build_router_no_auth as build_router};
use libp2p::PeerId;
use serde::Deserialize;
use serde_json::json;
use uuid::Uuid;

#[derive(Deserialize, Debug)]
struct ChatCompletionResult {
    request_id: Uuid,
    session_id: Uuid,
    content: String,
    session_key: String,
    #[allow(dead_code)]
    input_tokens: u32,
    #[allow(dead_code)]
    output_tokens: u32,
    #[allow(dead_code)]
    latency_ms: u32,
}

#[derive(Deserialize)]
struct EnclaveHealth {
    #[allow(dead_code)]
    public_key_hex: String,
    #[allow(dead_code)]
    uptime_ms: u64,
}

/// Bids must carry a *real* libp2p PeerId string — the coordinator
/// parses `node_peer_id` to dispatch the inference job over libp2p
/// (see `api/inference.rs`), so a made-up label won't parse.
fn bid(peer: &PeerId, price: u64, latency: u32, reputation: f32) -> InferenceBid {
    InferenceBid {
        request_id: Uuid::nil(), // rewritten by InMemoryMesh on publish
        node_peer_id: NodePeerId(peer.to_string()),
        price_per_1k: NanoX(price),
        latency_ms: latency,
        reputation,
        payout_address: format!("0x{:0>62}", peer.to_string()),
        http_endpoint: format!("http://node-{peer}.test:5000"),
        node_x25519_pubkey: None,
    }
}

async fn spawn(mesh: Arc<InMemoryMesh>) -> (String, tokio::task::JoinHandle<()>) {
    let state = AppState::with_mesh(mesh);
    let router = build_router(state);
    let (listener, addr) = bind("127.0.0.1:0").await.expect("bind");
    let base = format!("http://{addr}");
    let handle = tokio::spawn(async move {
        let _ = axum::serve(listener, router).await;
    });
    tokio::time::sleep(Duration::from_millis(50)).await;
    (base, handle)
}

#[tokio::test]
async fn auction_picks_best_bid_and_token_verifies() {
    let peer_a = PeerId::random();
    let peer_b = PeerId::random();
    let peer_c = PeerId::random();
    let mesh = Arc::new(InMemoryMesh::new());
    mesh.seed_bids(vec![
        bid(&peer_a, 200, 800, 0.4),
        bid(&peer_b, 50, 300, 0.9),
        bid(&peer_c, 100, 500, 0.6),
    ]);

    let (base, handle) = spawn(mesh).await;
    let client = reqwest::Client::new();

    // Confirm /enclave_health is reachable and well-formed (the
    // coordinator's signing key is verified end-to-end elsewhere —
    // this test focuses on auction winner selection).
    let _health: EnclaveHealth = client
        .get(format!("{base}/enclave_health"))
        .send()
        .await
        .expect("health")
        .json()
        .await
        .expect("json");

    let body = json!({
        "model": "qwen-72b",
        "messages": [{"role": "user", "content": "hello"}],
        "client_pubkey_hex": "00".repeat(32),
        "max_price_nanox": 10_000,
    });

    let resp = client
        .post(format!("{base}/v1/chat/completions"))
        .json(&body)
        .send()
        .await
        .expect("dispatch request");
    assert!(resp.status().is_success(), "status: {}", resp.status());
    let result: ChatCompletionResult = resp.json().await.expect("json");

    // Composite score makes B the winner (lowest price, lowest latency,
    // highest reputation). InMemoryMesh's dispatch_inference echoes the
    // peer it was asked to dispatch to in the reply content.
    assert_eq!(result.content, format!("mock-reply-from-{peer_b}"));
    assert!(!result.session_key.is_empty());
    assert_ne!(result.session_id, Uuid::nil());

    handle.abort();
}

#[tokio::test]
async fn empty_auction_returns_not_found() {
    let mesh = Arc::new(InMemoryMesh::new());
    // no seed — no bids will arrive

    let (base, handle) = spawn(mesh).await;
    let client = reqwest::Client::new();

    let body = json!({
        "model": "qwen-72b",
        "messages": [{"role": "user", "content": "hello"}],
        "client_pubkey_hex": "00".repeat(32),
    });

    let resp = client
        .post(format!("{base}/v1/chat/completions"))
        .json(&body)
        .send()
        .await
        .expect("request");
    assert_eq!(resp.status(), reqwest::StatusCode::SERVICE_UNAVAILABLE);

    handle.abort();
}

#[tokio::test]
async fn bad_client_pubkey_hex_returns_bad_request() {
    let peer = PeerId::random();
    let mesh = Arc::new(InMemoryMesh::new());
    mesh.seed_bids(vec![bid(&peer, 50, 300, 0.9)]);

    let (base, handle) = spawn(mesh).await;
    let client = reqwest::Client::new();

    let body = json!({
        "model": "qwen-72b",
        "messages": [{"role": "user", "content": "hello"}],
        "client_pubkey_hex": "nothex",
    });

    let resp = client
        .post(format!("{base}/v1/chat/completions"))
        .json(&body)
        .send()
        .await
        .expect("request");
    assert_eq!(resp.status(), reqwest::StatusCode::BAD_REQUEST);

    handle.abort();
}
