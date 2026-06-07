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
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InferenceBid {
    pub request_id: RequestId,
    pub node_peer_id: NodePeerId,
    pub price_per_1k: NanoX,
    pub latency_ms: u32,
    pub reputation: f32,
    /// HTTP endpoint the client will dial after the coordinator picks
    /// this bid. The node advertises whatever URL it wants the client
    /// to use (typically `http://<public_ip>:<port>`).
    pub http_endpoint: String,
    /// Sui address where the on-chain vault should disburse this
    /// node's share if it wins and serves the request. Required for
    /// `vault::settle` to be able to pay this peer.
    pub payout_address: String,
    /// X25519 public key the node uses for prompt encryption.
    /// Nodes that support encrypted prompts set this; `None` means
    /// the node accepts plaintext only. The coordinator copies this
    /// into the `DispatchToken` so the client can ECDH-encrypt the
    /// prompt before posting to `node_url/v1/inference`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub node_x25519_pubkey: Option<[u8; 32]>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NodeCapabilities {
    pub peer_id: NodePeerId,
    pub models: Vec<String>,
    pub max_concurrent_jobs: u32,
    // Deliberately NO `tee_enabled` field.
}
