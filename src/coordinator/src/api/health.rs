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
    pub uptime_ms: u64,
}

pub async fn enclave_health(State(state): State<AppState>) -> Json<EnclaveHealthResponse> {
    Json(EnclaveHealthResponse {
        public_key_hex: hex::encode(state.enclave_pubkey_bytes()),
        uptime_ms: state.uptime_ms(),
    })
}

pub async fn get_attestation(State(state): State<AppState>) -> Json<AttestationDoc> {
    let pubkey = state.enclave_pubkey_bytes();
    // Empty nonce is acceptable in the mock path; production code will
    // accept a client-supplied nonce as a query parameter.
    Json(nautilus_enclave::get_attestation(&pubkey, &[]))
}
