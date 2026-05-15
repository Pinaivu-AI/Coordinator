//! In-enclave `HashMap<PeerId, ReputationEntry>` with TTL. Rebuilt
//! from gossip on restart; never persisted to disk.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReputationEntry {
    pub merkle_root: [u8; 32],
    pub score: f32,
    pub success_rate: f32,
    pub avg_latency_ms: u32,
    pub verified_proofs: u64,
    pub seen_at_ms: u64,
}
