//! `DispatchJob` — the unit of work tracked per accepted inference
//! request. Written to Postgres on dispatch, read by the deadline
//! watcher to fire refunds on timeout.

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

impl JobStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            JobStatus::Dispatched => "Dispatched",
            JobStatus::Acked => "Acked",
            JobStatus::Completed => "Completed",
            JobStatus::TimedOut => "TimedOut",
            JobStatus::Refunded => "Refunded",
        }
    }
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
