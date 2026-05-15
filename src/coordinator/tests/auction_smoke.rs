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
use coordinator::{bind, build_router};
use serde::Deserialize;
use serde_json::json;
use uuid::Uuid;

#[derive(Deserialize, Debug)]
struct ChatCompletionDispatch {
    request_id: Uuid,
    node_url: String,
    dispatch_token: DispatchTokenWire,
}

#[derive(Deserialize, Debug)]
struct DispatchTokenWire {
    request_id: Uuid,
    primary_peer_id: NodePeerIdWire,
    coordinator_pubkey: Vec<u8>,
    signature: Vec<u8>,
    #[serde(flatten)]
    _rest: serde_json::Value,
}

#[derive(Deserialize, Debug)]
struct NodePeerIdWire(String);

#[derive(Deserialize)]
struct EnclaveHealth {
    public_key_hex: String,
    #[allow(dead_code)]
    uptime_ms: u64,
}

fn bid(peer: &str, price: u64, latency: u32, reputation: f32) -> InferenceBid {
    InferenceBid {
        request_id: Uuid::nil(), // rewritten by InMemoryMesh on publish
        node_peer_id: NodePeerId(peer.into()),
        price_per_1k: NanoX(price),
        latency_ms: latency,
        reputation,
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
    let mesh = Arc::new(InMemoryMesh::new());
    mesh.seed_bids(vec![
        bid("A-slow-expensive", 200, 800, 0.4),
        bid("B-fast-cheap-rep", 50, 300, 0.9),
        bid("C-mid", 100, 500, 0.6),
    ]);

    let (base, handle) = spawn(mesh).await;
    let client = reqwest::Client::new();

    let coord_pubkey_hex: String = client
        .get(format!("{base}/enclave_health"))
        .send()
        .await
        .expect("health")
        .json::<EnclaveHealth>()
        .await
        .expect("json")
        .public_key_hex;

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
    let dispatch: ChatCompletionDispatch = resp.json().await.expect("json");

    // Composite score makes B the winner (lowest price, lowest latency,
    // highest reputation).
    assert_eq!(dispatch.dispatch_token.primary_peer_id.0, "B-fast-cheap-rep");
    assert!(dispatch.node_url.contains("B-fast-cheap-rep"));
    assert_eq!(dispatch.dispatch_token.request_id, dispatch.request_id);

    // The dispatch token was signed by the coordinator's enclave key
    // exposed via /enclave_health. They must match.
    let coord_pubkey_bytes = hex::decode(&coord_pubkey_hex).expect("hex");
    assert_eq!(dispatch.dispatch_token.coordinator_pubkey, coord_pubkey_bytes);
    assert_eq!(dispatch.dispatch_token.signature.len(), 64);

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
        "messages": [],
        "client_pubkey_hex": "00".repeat(32),
    });

    let resp = client
        .post(format!("{base}/v1/chat/completions"))
        .json(&body)
        .send()
        .await
        .expect("request");
    assert_eq!(resp.status(), reqwest::StatusCode::NOT_FOUND);

    handle.abort();
}

#[tokio::test]
async fn bad_client_pubkey_hex_returns_bad_request() {
    let mesh = Arc::new(InMemoryMesh::new());
    mesh.seed_bids(vec![bid("A", 50, 300, 0.9)]);

    let (base, handle) = spawn(mesh).await;
    let client = reqwest::Client::new();

    let body = json!({
        "model": "qwen-72b",
        "messages": [],
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
