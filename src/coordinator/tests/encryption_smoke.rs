//! Integration tests for the prompt-encryption path.
//!
//! Spawns the coordinator, fetches its X25519 pubkey from
//! `GET /enclave_health`, performs the ECDH + AES-256-GCM encryption a
//! real client would do, and verifies the coordinator decrypts the
//! messages and returns a valid dispatch token.
//!
//! Run with:
//!   cargo test -p coordinator --test encryption_smoke

use std::sync::Arc;
use std::time::Duration;

use aes_gcm::{
    Aes256Gcm, Key,
    aead::{Aead, AeadCore, KeyInit},
};
use base64::{Engine as _, engine::general_purpose::STANDARD as BASE64};
use coordinator::app::AppState;
use coordinator::mesh::InMemoryMesh;
use coordinator::protocol::{InferenceBid, NanoX, NodePeerId};
use coordinator::{bind, build_router};
use rand::rngs::OsRng;
use serde::Deserialize;
use serde_json::json;
use sha2::{Digest, Sha256};
use uuid::Uuid;
use x25519_dalek::{EphemeralSecret, PublicKey as X25519PublicKey};

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

#[derive(Deserialize)]
struct EnclaveHealth {
    x25519_pubkey_hex: String,
}

#[derive(Deserialize, Debug)]
struct ChatCompletionDispatch {
    #[allow(dead_code)]
    request_id: Uuid,
    node_url:   String,
    dispatch_token: serde_json::Value,
}

fn one_bidder_mesh() -> Arc<InMemoryMesh> {
    let mesh = Arc::new(InMemoryMesh::new());
    mesh.seed_bids(vec![InferenceBid {
        request_id:    Uuid::nil(),
        node_peer_id:  NodePeerId("enc-test-node".into()),
        price_per_1k:  NanoX(50),
        latency_ms:    200,
        reputation:    0.9,
        payout_address: "0xenc".into(),
        http_endpoint: "http://enc-node.test:5000".into(),
    }]);
    mesh
}

async fn spawn(mesh: Arc<InMemoryMesh>) -> (String, tokio::task::JoinHandle<()>) {
    let state = AppState::with_mesh(mesh);
    let router = build_router(state);
    let (listener, addr) = bind("127.0.0.1:0").await.expect("bind ephemeral");
    let base = format!("http://{addr}");
    let handle = tokio::spawn(async move {
        let _ = axum::serve(listener, router).await;
    });
    tokio::time::sleep(Duration::from_millis(50)).await;
    (base, handle)
}

/// Perform the full client-side ECDH + AES-256-GCM encryption.
///
/// Mirrors exactly what a client SDK should do:
/// 1. Parse the enclave's X25519 public key.
/// 2. Generate a fresh ephemeral X25519 keypair.
/// 3. ECDH → 32-byte shared secret.
/// 4. `SHA-256("pinaivu-aes-key-v1" ‖ shared)` → AES key.
/// 5. AES-256-GCM encrypt the JSON-serialized messages.
///
/// Returns `(client_pub_hex, ciphertext_b64, nonce_b64)`.
fn client_encrypt(
    enclave_x25519_hex: &str,
    messages: &serde_json::Value,
) -> (String, String, String) {
    let enclave_pub_bytes: [u8; 32] = hex::decode(enclave_x25519_hex)
        .expect("hex")
        .try_into()
        .expect("32 bytes");
    let enclave_pub = X25519PublicKey::from(enclave_pub_bytes);

    let client_priv = EphemeralSecret::random_from_rng(OsRng);
    let client_pub  = X25519PublicKey::from(&client_priv);

    let shared = client_priv.diffie_hellman(&enclave_pub).to_bytes();

    let mut h = Sha256::new();
    h.update(b"pinaivu-aes-key-v1");
    h.update(shared);
    let aes_key: [u8; 32] = h.finalize().into();

    let plaintext  = serde_json::to_vec(messages).expect("json");
    let key        = Key::<Aes256Gcm>::from_slice(&aes_key);
    let cipher     = Aes256Gcm::new(key);
    let nonce      = Aes256Gcm::generate_nonce(&mut OsRng);
    let ciphertext = cipher.encrypt(&nonce, plaintext.as_ref()).expect("encrypt");

    (
        hex::encode(client_pub.as_bytes()),
        BASE64.encode(&ciphertext),
        BASE64.encode(&nonce),
    )
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

/// Golden path: encrypted messages → coordinator decrypts inside enclave →
/// auction runs → signed dispatch token returned.
#[tokio::test]
async fn encrypted_request_returns_valid_dispatch_token() {
    let (base, handle) = spawn(one_bidder_mesh()).await;
    let client = reqwest::Client::new();

    // 1. Fetch the enclave's X25519 public key.
    let health: EnclaveHealth = client
        .get(format!("{base}/enclave_health"))
        .send().await.unwrap()
        .json().await.unwrap();
    assert_eq!(health.x25519_pubkey_hex.len(), 64, "x25519 pubkey must be 32 bytes hex");

    // 2. Encrypt messages (client-side ECDH).
    let messages = json!([{"role": "user", "content": "what is 2 + 2?"}]);
    let (client_pub_hex, enc_b64, nonce_b64) =
        client_encrypt(&health.x25519_pubkey_hex, &messages);

    // 3. POST with encrypted payload.
    let resp = client
        .post(format!("{base}/v1/chat/completions"))
        .json(&json!({
            "model": "qwen-72b",
            "client_pubkey_hex": "00".repeat(32),
            "client_x25519_pubkey_hex": client_pub_hex,
            "messages_encrypted": enc_b64,
            "messages_nonce":     nonce_b64,
        }))
        .send().await.unwrap();

    assert!(resp.status().is_success(), "unexpected status: {}", resp.status());
    let dispatch: ChatCompletionDispatch = resp.json().await.unwrap();
    assert_eq!(dispatch.dispatch_token["primary_peer_id"], "enc-test-node");
    assert_eq!(dispatch.node_url, "http://enc-node.test:5000");

    handle.abort();
}

/// Plaintext mode must still work — the three encrypted fields are optional.
#[tokio::test]
async fn plaintext_request_still_works() {
    let (base, handle) = spawn(one_bidder_mesh()).await;
    let client = reqwest::Client::new();

    let resp = client
        .post(format!("{base}/v1/chat/completions"))
        .json(&json!({
            "model": "qwen-72b",
            "messages": [{"role": "user", "content": "hello"}],
            "client_pubkey_hex": "00".repeat(32),
        }))
        .send().await.unwrap();

    assert!(resp.status().is_success(), "status: {}", resp.status());
    let dispatch: ChatCompletionDispatch = resp.json().await.unwrap();
    assert_eq!(dispatch.dispatch_token["primary_peer_id"], "enc-test-node");

    handle.abort();
}

/// Only one of the three encrypted fields present → 400 Bad Request.
#[tokio::test]
async fn partial_encrypted_fields_rejected() {
    let (base, handle) = spawn(one_bidder_mesh()).await;
    let client = reqwest::Client::new();

    let resp = client
        .post(format!("{base}/v1/chat/completions"))
        .json(&json!({
            "model": "qwen-72b",
            "client_pubkey_hex": "00".repeat(32),
            // only client_x25519_pubkey_hex — missing enc + nonce
            "client_x25519_pubkey_hex": "ab".repeat(32),
        }))
        .send().await.unwrap();

    assert_eq!(resp.status(), reqwest::StatusCode::BAD_REQUEST);

    handle.abort();
}

/// Ciphertext bit-flipped → AES-GCM authentication fails → 400.
#[tokio::test]
async fn corrupted_ciphertext_rejected() {
    let (base, handle) = spawn(one_bidder_mesh()).await;
    let client = reqwest::Client::new();

    let health: EnclaveHealth = client
        .get(format!("{base}/enclave_health"))
        .send().await.unwrap()
        .json().await.unwrap();

    let messages = json!([{"role": "user", "content": "tamper me"}]);
    let (client_pub_hex, enc_b64, nonce_b64) =
        client_encrypt(&health.x25519_pubkey_hex, &messages);

    // Flip the last byte of the ciphertext to break the auth tag.
    let mut ct = BASE64.decode(&enc_b64).unwrap();
    *ct.last_mut().unwrap() ^= 0xff;

    let resp = client
        .post(format!("{base}/v1/chat/completions"))
        .json(&json!({
            "model": "qwen-72b",
            "client_pubkey_hex": "00".repeat(32),
            "client_x25519_pubkey_hex": client_pub_hex,
            "messages_encrypted": BASE64.encode(&ct),
            "messages_nonce":     nonce_b64,
        }))
        .send().await.unwrap();

    assert_eq!(resp.status(), reqwest::StatusCode::BAD_REQUEST);

    handle.abort();
}

/// Client encrypted with a random key (wrong enclave pubkey) → decrypt fails → 400.
#[tokio::test]
async fn wrong_key_ciphertext_rejected() {
    let (base, handle) = spawn(one_bidder_mesh()).await;
    let client = reqwest::Client::new();

    let health: EnclaveHealth = client
        .get(format!("{base}/enclave_health"))
        .send().await.unwrap()
        .json().await.unwrap();

    let messages = json!([{"role": "user", "content": "wrong key"}]);

    // Encrypt with the correct client pubkey but wrong enclave pubkey.
    let (client_pub_hex, _correct_enc, correct_nonce) =
        client_encrypt(&health.x25519_pubkey_hex, &messages);

    // Encrypt the payload against a random enclave key — mismatch.
    let random_pub_hex = hex::encode([0xde_u8; 32]);
    let (_, wrong_enc, _) = client_encrypt(&random_pub_hex, &messages);

    let resp = client
        .post(format!("{base}/v1/chat/completions"))
        .json(&json!({
            "model": "qwen-72b",
            "client_pubkey_hex": "00".repeat(32),
            "client_x25519_pubkey_hex": client_pub_hex,
            "messages_encrypted": wrong_enc,
            "messages_nonce":     correct_nonce,
        }))
        .send().await.unwrap();

    assert_eq!(resp.status(), reqwest::StatusCode::BAD_REQUEST);

    handle.abort();
}

/// Non-base64 nonce → 400 before decryption even starts.
#[tokio::test]
async fn invalid_nonce_base64_rejected() {
    let (base, handle) = spawn(one_bidder_mesh()).await;
    let client = reqwest::Client::new();

    let health: EnclaveHealth = client
        .get(format!("{base}/enclave_health"))
        .send().await.unwrap()
        .json().await.unwrap();

    let messages = json!([{"role": "user", "content": "bad nonce"}]);
    let (client_pub_hex, enc_b64, _) =
        client_encrypt(&health.x25519_pubkey_hex, &messages);

    let resp = client
        .post(format!("{base}/v1/chat/completions"))
        .json(&json!({
            "model": "qwen-72b",
            "client_pubkey_hex": "00".repeat(32),
            "client_x25519_pubkey_hex": client_pub_hex,
            "messages_encrypted": enc_b64,
            "messages_nonce": "!!!not-base64!!!",
        }))
        .send().await.unwrap();

    assert_eq!(resp.status(), reqwest::StatusCode::BAD_REQUEST);

    handle.abort();
}
