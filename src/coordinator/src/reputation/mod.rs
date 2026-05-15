//! Reputation — the coordinator subscribes to the reputation gossip
//! topic and caches each peer's latest Merkle root + score. It does
//! not author its own reputation tree.

pub mod cache;
pub mod verify;
