//! End-to-end test of the libp2p mesh.
//!
//! Spawns a coordinator (with `Libp2pMesh`) and a `MockNode` (a bare
//! libp2p participant that subscribes to `/pinaivu/inference/any` and
//! publishes a bid in reply to any request it observes). Dials the
//! two together, waits for gossipsub mesh formation, then runs a
//! chat-completions request and verifies the auction returns the
//! mock node as the dispatch target.

use std::sync::Arc;
use std::time::Duration;

use anyhow::Result;
use coordinator::app::AppState;
use coordinator::mesh::{
    behaviour::{libp2p_identity_from_ed25519_secret, PinaivuBehaviour, PinaivuBehaviourEvent},
    inference_proto::InferenceReply,
    spawn_libp2p_mesh,
    topics::{BIDS, INFERENCE_ANY},
    PeerRegistry,
};
use coordinator::receipts::InMemoryReceiptArchive;
use coordinator::protocol::{InferenceBid, InferenceRequest, NanoX, NodePeerId};
use coordinator::{bind, build_router_no_auth as build_router};
use futures::StreamExt;
use libp2p::{
    gossipsub::{self, IdentTopic},
    multiaddr::Protocol,
    request_response,
    swarm::SwarmEvent,
    Multiaddr, PeerId,
};
use rand::{rngs::OsRng, RngCore};
use serde::Deserialize;
use serde_json::json;
use uuid::Uuid;

#[derive(Deserialize, Debug)]
struct ChatCompletionResult {
    #[allow(dead_code)]
    request_id: Uuid,
    #[allow(dead_code)]
    session_id: Uuid,
    content: String,
    #[allow(dead_code)]
    session_key: String,
}

/// Bare libp2p participant that listens on the marketplace network
/// and publishes a configured bid in reply to every inference
/// request it observes.
struct MockNode {
    listen_addr: Multiaddr,
    #[allow(dead_code)]
    peer_id: PeerId,
    _task: tokio::task::JoinHandle<()>,
}

/// Parameters for the bid the mock node publishes. `node_peer_id` is
/// filled in once the swarm's real `PeerId` is known — the coordinator
/// parses that field to dispatch the inference job over libp2p, so it
/// must be a real, connectable PeerId, not a made-up label.
struct MockBidParams {
    price_per_1k: u64,
    latency_ms: u32,
    reputation: f32,
    http_endpoint: String,
    payout_address: String,
}

async fn spawn_mock_node(params: MockBidParams) -> Result<MockNode> {
    let mut secret = [0u8; 32];
    OsRng.fill_bytes(&mut secret);
    let identity = libp2p_identity_from_ed25519_secret(&secret)?;

    let mut swarm = libp2p::SwarmBuilder::with_existing_identity(identity)
        .with_tokio()
        .with_tcp(
            libp2p::tcp::Config::default(),
            libp2p::noise::Config::new,
            libp2p::yamux::Config::default,
        )?
        .with_behaviour(|key| {
            PinaivuBehaviour::new(key)
                .map_err(|e| Box::<dyn std::error::Error + Send + Sync>::from(e.to_string()))
        })?
        .with_swarm_config(|c| c.with_idle_connection_timeout(Duration::from_secs(60)))
        .build();

    let local_peer_id = *swarm.local_peer_id();
    let bid_template = InferenceBid {
        request_id: Uuid::nil(),
        node_peer_id: NodePeerId(local_peer_id.to_string()),
        price_per_1k: NanoX(params.price_per_1k),
        latency_ms: params.latency_ms,
        reputation: params.reputation,
        http_endpoint: params.http_endpoint,
        payout_address: params.payout_address,
        node_x25519_pubkey: None,
    };

    let topics = [INFERENCE_ANY, BIDS];
    for t in topics {
        swarm
            .behaviour_mut()
            .gossipsub
            .subscribe(&IdentTopic::new(t))?;
    }
    swarm.listen_on("/ip4/127.0.0.1/tcp/0".parse()?)?;

    // Drive the swarm until we observe our first listen address so
    // we can hand a dialable multiaddr back to the caller.
    let listen_addr = loop {
        match swarm.select_next_some().await {
            SwarmEvent::NewListenAddr { address, .. } => {
                break address.with(Protocol::P2p(local_peer_id));
            }
            _ => continue,
        }
    };

    let task = tokio::spawn(async move {
        loop {
            match swarm.select_next_some().await {
                SwarmEvent::Behaviour(PinaivuBehaviourEvent::Gossipsub(
                    gossipsub::Event::Message { message, .. },
                )) => {
                    if message.topic == IdentTopic::new(INFERENCE_ANY).hash() {
                        if let Ok(req) = serde_json::from_slice::<InferenceRequest>(&message.data) {
                            let mut bid = bid_template.clone();
                            bid.request_id = req.request_id;
                            let payload = serde_json::to_vec(&bid).unwrap();
                            let _ = swarm
                                .behaviour_mut()
                                .gossipsub
                                .publish(IdentTopic::new(BIDS), payload);
                        }
                    }
                }
                SwarmEvent::Behaviour(PinaivuBehaviourEvent::Inference(
                    request_response::Event::Message {
                        message: request_response::Message::Request { request, channel, .. },
                        ..
                    },
                )) => {
                    // Echo the dispatch back as a canned reply — proves
                    // the coordinator successfully dispatched the actual
                    // inference job over libp2p (not just the auction).
                    let reply = InferenceReply {
                        request_id: request.dispatch_token.request_id,
                        session_id: request.dispatch_token.session_id,
                        content: "mock-node-reply".into(),
                        input_tokens: 5,
                        output_tokens: 7,
                        latency_ms: 3,
                        error: None,
                    };
                    let _ = swarm.behaviour_mut().inference.send_response(channel, reply);
                }
                _ => {}
            }
        }
    });

    Ok(MockNode {
        listen_addr,
        peer_id: local_peer_id,
        _task: task,
    })
}

#[tokio::test]
async fn coordinator_auctions_a_real_bid_over_libp2p() {
    // Spawn the mock node first so the coordinator has something to
    // dial. Lower price + latency + higher reputation than anything
    // the coordinator would invent on its own, so the dispatch
    // unambiguously names this node.
    let mock = spawn_mock_node(MockBidParams {
        price_per_1k: 50,
        latency_ms: 200,
        reputation: 0.95,
        http_endpoint: "http://mock-node-primary.test:5000".into(),
        payout_address: "0xMOCK-NODE-PRIMARY-payout".into(),
    })
    .await
    .expect("mock node");

    // Spawn the coordinator's libp2p mesh on an ephemeral loopback
    // port. Single enclave keypair shared between libp2p and HTTP.
    let enclave_key = Arc::new(nautilus_enclave::EnclaveKeyPair::generate());
    let registry = Arc::new(PeerRegistry::new(Duration::from_secs(60)));
    let mesh_handle = spawn_libp2p_mesh(
        enclave_key.clone(),
        "/ip4/127.0.0.1/tcp/0".parse().unwrap(),
        registry.clone(),
        Arc::new(InMemoryReceiptArchive::new()),
    )
    .await
    .expect("spawn libp2p mesh");

    // Dial the mock node from the coordinator.
    mesh_handle
        .mesh
        .dial(mock.listen_addr.clone())
        .await
        .expect("dial mock node");

    // Wait long enough for gossipsub heartbeats (default 1s) to
    // propagate topic subscriptions and GRAFT both peers into each
    // other's meshes. 2.5s is comfortable headroom.
    tokio::time::sleep(Duration::from_millis(2500)).await;

    // Stand up the HTTP surface backed by the same mesh.
    let state = AppState::with_mesh_and_registry(mesh_handle.mesh.clone(), registry);
    let router = build_router(state);
    let (listener, addr) = bind("127.0.0.1:0").await.expect("bind http");
    let base = format!("http://{addr}");
    let http_task = tokio::spawn(async move {
        let _ = axum::serve(listener, router).await;
    });
    tokio::time::sleep(Duration::from_millis(50)).await;

    let body = json!({
        "model": "qwen-72b",
        "messages": [{"role": "user", "content": "hello"}],
        "client_pubkey_hex": "00".repeat(32),
        "max_price_nanox": 10_000,
    });

    let client = reqwest::Client::new();
    // Bump the per-request timeout so the gossipsub trip (auction)
    // plus the inference request-response round trip aren't racing
    // CI host load; 8s is generous for two loopback round trips.
    let resp = client
        .post(format!("{base}/v1/chat/completions"))
        .json(&body)
        .timeout(Duration::from_secs(8))
        .send()
        .await
        .expect("post chat completions");

    assert!(
        resp.status().is_success(),
        "status={} body={:?}",
        resp.status(),
        resp.text().await.ok()
    );
    let result: ChatCompletionResult = resp.json().await.expect("json");

    // The mock node's canned reply proves the coordinator successfully
    // dispatched the actual inference job over libp2p to the winning
    // peer (not just ran the auction).
    assert_eq!(result.content, "mock-node-reply");

    http_task.abort();
    drop(mesh_handle);
    drop(mock);
}
