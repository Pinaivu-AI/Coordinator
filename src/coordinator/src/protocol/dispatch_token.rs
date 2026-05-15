//! Signed dispatch token.
//!
//! Issued by the coordinator to a client after auction. The client
//! forwards it to the chosen primary node, which verifies the
//! coordinator's signature before serving the request.

use serde::{Deserialize, Serialize};

use super::types::{NanoX, NodePeerId, RequestId};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DispatchToken {
    pub request_id: RequestId,
    pub client_pubkey: [u8; 32],
    pub primary_peer_id: NodePeerId,
    pub settlement_id: String,
    pub max_price_nanox: NanoX,
    pub issued_at_ms: u64,
    pub deadline_ms: u64,
    pub coordinator_pubkey: [u8; 32],
    pub signature: Vec<u8>,
}

impl DispatchToken {
    pub fn canonical_bytes(&self) -> Vec<u8> {
        Vec::new()
    }

    pub fn verify(&self) -> bool {
        false
    }
}
