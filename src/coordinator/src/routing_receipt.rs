//! Signed routing receipt — the post-completion audit artefact for an
//! inference job. Holders of `(receipt, coordinator_pubkey)` can verify
//! offline that the coordinator routed `request_id` to the recorded
//! peers and observed the listed proofs.

use serde::{Deserialize, Serialize};

use crate::types::{NodePeerId, RequestId};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RoutingReceipt {
    pub request_id: RequestId,
    pub client_id: String,
    pub primary_peer_id: NodePeerId,
    pub helper_peer_ids: Vec<NodePeerId>,
    pub bid_set_hash: [u8; 32],
    pub proof_ids: Vec<[u8; 32]>,
    pub aggregated_output_hash: [u8; 32],
    pub timestamp_ms: u64,
    pub coordinator_pubkey: [u8; 32],
    pub signature: Vec<u8>,
}

impl RoutingReceipt {
    pub fn canonical_bytes(&self) -> Vec<u8> {
        Vec::new()
    }

    pub fn verify(&self) -> bool {
        false
    }
}
