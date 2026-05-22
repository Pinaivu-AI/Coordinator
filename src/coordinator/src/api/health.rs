//! Liveness + attestation endpoints.
//!
//!   `GET /health`          — process liveness
//!   `GET /enclave_health`  — coordinator pubkey + uptime
//!   `GET /get_attestation` — NSM document binding pubkey to PCRs

use axum::{extract::State, Json};
use nautilus_enclave::AttestationDoc;
use serde::Serialize;

use crate::app::AppState;

pub async fn health() -> &'static str {
    "ok"
}

#[derive(Serialize)]
pub struct EnclaveHealthResponse {
    pub public_key_hex: String,
    /// X25519 public key derived from the same enclave seed.
    /// Clients use this to set up an ECDH session before encrypting
    /// their messages for `POST /v1/chat/completions`.
    pub x25519_pubkey_hex: String,
    pub peer_id: Option<String>,
    pub uptime_ms: u64,
    /// Set once the background registration task completes; `null` until then.
    pub enclave_object_id: Option<String>,
    pub sui_tx_digest: Option<String>,
}

pub async fn enclave_health(State(state): State<AppState>) -> Json<EnclaveHealthResponse> {
    let pubkey = state.enclave_pubkey_bytes();
    let x25519_pubkey = state.enclave_key().x25519_public_key();
    let on_chain = state.on_chain().read().await;
    Json(EnclaveHealthResponse {
        public_key_hex: hex::encode(pubkey),
        x25519_pubkey_hex: hex::encode(x25519_pubkey.as_bytes()),
        peer_id: peer_id_from_ed25519(&pubkey),
        uptime_ms: state.uptime_ms(),
        enclave_object_id: on_chain.as_ref().map(|r| r.enclave_object_id.clone()),
        sui_tx_digest: on_chain.as_ref().map(|r| r.tx_digest.clone()),
    })
}

/// Derive the libp2p `PeerId` string from a raw Ed25519 public key.
/// Matches the identity the coordinator uses when it builds its libp2p
/// swarm — operators need this to build the node's `--coordinator-addr`
/// multiaddr without having to scrape log files.
fn peer_id_from_ed25519(bytes: &[u8; 32]) -> Option<String> {
    let ed = libp2p::identity::ed25519::PublicKey::try_from_bytes(bytes).ok()?;
    let pk: libp2p::identity::PublicKey = ed.into();
    Some(pk.to_peer_id().to_string())
}

pub async fn get_attestation(
    State(state): State<AppState>,
) -> Result<Json<AttestationDoc>, crate::app::AppError> {
    let pubkey = state.enclave_pubkey_bytes();
    // Empty nonce for now; production should accept a client-supplied
    // nonce as a query parameter and bind it into the attestation doc.
    let doc = nautilus_enclave::get_attestation(&pubkey, &[])
        .map_err(|e| crate::app::AppError::Internal(format!("attestation: {e}")))?;
    Ok(Json(doc))
}
