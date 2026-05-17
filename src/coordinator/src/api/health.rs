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
    pub peer_id: Option<String>,
    pub uptime_ms: u64,
}

pub async fn enclave_health(State(state): State<AppState>) -> Json<EnclaveHealthResponse> {
    let pubkey = state.enclave_pubkey_bytes();
    Json(EnclaveHealthResponse {
        public_key_hex: hex::encode(pubkey),
        peer_id: peer_id_from_ed25519(&pubkey),
        uptime_ms: state.uptime_ms(),
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

pub async fn get_attestation(State(state): State<AppState>) -> Json<AttestationDoc> {
    let pubkey = state.enclave_pubkey_bytes();
    // Empty nonce is acceptable in the mock path; production code will
    // accept a client-supplied nonce as a query parameter.
    Json(nautilus_enclave::get_attestation(&pubkey, &[]))
}
