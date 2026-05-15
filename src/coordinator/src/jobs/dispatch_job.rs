//! `DispatchJob` — the unit of work tracked by apalis. Records the
//! primary peer, the deadline, the current status, and the opaque
//! escrow handle so the worker can refund on timeout.

use serde::{Deserialize, Serialize};

use crate::protocol::{NodePeerId, RequestId};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum JobStatus {
    Dispatched,
    Acked,
    Completed,
    TimedOut,
    Refunded,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DispatchJob {
    pub request_id: RequestId,
    pub primary_peer_id: NodePeerId,
    pub dispatched_at_ms: u64,
    pub deadline_ms: u64,
    pub status: JobStatus,
    pub escrow_handle_json: String,
}
