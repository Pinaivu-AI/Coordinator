//! libp2p request-response protocol carrying the actual inference job:
//!   coordinator → node : [`InferenceDispatch`] (dispatch token + prompt)
//!   node → coordinator : [`InferenceReply`] (the model's output)
//!
//! This rides the same connection the node already opened *outbound*
//! to the coordinator (to join the mesh), so it works through NAT with
//! no extra setup — unlike the HTTP `node_url` path, which requires
//! something to dial *into* the node, which fails for any node that
//! isn't publicly reachable (the common case for home/laptop GPUs).
//!
//! Protocol id: `/pinaivu/inference/1.0.0`. CBOR-encoded over the wire.

use libp2p::StreamProtocol;
use serde::{Deserialize, Serialize};

use crate::dispatch_token::DispatchToken;
use crate::types::{RequestId, SessionId};

pub const INFERENCE_PROTOCOL: StreamProtocol = StreamProtocol::new("/pinaivu/inference/1.0.0");

/// Sent by the coordinator to the winning node after the auction.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InferenceDispatch {
    pub dispatch_token: DispatchToken,
    /// AES-256 key (base64) so the node can decrypt the Walrus session
    /// blob. Empty string for stateless nodes/turns.
    #[serde(default)]
    pub session_key: String,
    pub new_user_message: String,
    /// Cross-session memory facts recalled by chat-relayer's own
    /// pgvector + Walrus stack. Absent for direct API callers.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub memwal_context: Option<String>,
}

/// Returned by the node once inference completes (or fails).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InferenceReply {
    pub request_id: RequestId,
    pub session_id: SessionId,
    pub content: String,
    pub input_tokens: u32,
    pub output_tokens: u32,
    pub latency_ms: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}
