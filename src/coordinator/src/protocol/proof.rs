//! `ProofOfInference` — a node-signed execution receipt.
//!
//! Self-verifiable: a holder of `(proof, node_pubkey)` can verify the
//! signature offline with no network or chain access.

use serde::{Deserialize, Serialize};

use super::types::{NanoX, NodePeerId, RequestId, SessionId};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProofOfInference {
    pub request_id: RequestId,
    pub session_id: SessionId,
    pub node_peer_id: NodePeerId,
    pub client_address: String,
    pub model_id: String,
    pub input_tokens: u32,
    pub output_tokens: u32,
    pub latency_ms: u32,
    pub price_paid_nanox: NanoX,
    pub timestamp: u64,
    pub input_hash: [u8; 32],
    pub output_hash: [u8; 32],
    pub settlement_id: String,
    pub escrow_tx_id: Option<String>,
    pub node_pubkey: [u8; 32],
    pub signature: Vec<u8>,
}

impl ProofOfInference {
    /// Canonical bytes that the node signs. Excludes the `signature`
    /// field itself. Whitepaper §6.2.
    pub fn canonical_bytes(&self) -> Vec<u8> {
        // TODO: deterministic JSON serialisation of every field except `signature`.
        Vec::new()
    }

    /// SHA-256 of `canonical_bytes()` — Merkle leaf hash.
    pub fn id(&self) -> [u8; 32] {
        [0u8; 32]
    }

    /// Verify the embedded Ed25519 signature against `node_pubkey`.
    pub fn verify(&self) -> bool {
        false
    }
}
