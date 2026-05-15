//! Wire-format types shared between the coordinator, GPU nodes, and
//! clients. GPU nodes are standard hardware: no TEE-capability fields
//! appear on node-side types.

use serde::{Deserialize, Serialize};

pub type RequestId = uuid::Uuid;
pub type SessionId = uuid::Uuid;

/// Identifier for a libp2p peer. Wraps a string for now to avoid
/// pulling libp2p into every module that just needs to name a peer.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct NodePeerId(pub String);

/// Amount denominated in NanoX (1 X = 10^9 NanoX), per whitepaper §6.1.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct NanoX(pub u64);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum PrivacyLevel {
    Standard,
    Private,
    Fragmented,
    /// Routes through the attested coordinator and fragments across
    /// >=2 nodes. Note: does NOT require TEE on the GPU node.
    Maximum,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InferenceRequest {
    pub request_id: RequestId,
    pub session_id: SessionId,
    pub model: String,
    pub privacy: PrivacyLevel,
    // TODO: encrypted prompt, context blob id, budget,
    // accepted_settlements, client pubkey.
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InferenceBid {
    pub request_id: RequestId,
    pub node_peer_id: NodePeerId,
    pub price_per_1k: NanoX,
    pub latency_ms: u32,
    pub reputation: f32,
    // TODO: accepted_settlements, model info, capacity hints.
    // Deliberately NO `has_tee` field — GPU nodes are not TEE components.
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NodeCapabilities {
    pub peer_id: NodePeerId,
    pub models: Vec<String>,
    pub max_concurrent_jobs: u32,
    // Deliberately NO `tee_enabled` field.
}
