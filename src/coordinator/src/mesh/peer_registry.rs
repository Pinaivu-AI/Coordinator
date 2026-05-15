//! In-enclave snapshot of who's on the marketplace network.
//!
//! Populated by `/pinaivu/announce` gossip; queried at auction time
//! to validate bidders and decide which peers to dial. TTL-evicted so
//! peers that stop announcing fall out of the registry without any
//! explicit teardown signal.

use std::collections::HashMap;
use std::sync::RwLock;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use libp2p::{Multiaddr, PeerId};

use crate::protocol::NodeCapabilities;

#[derive(Debug, Clone)]
pub struct PeerEntry {
    pub capabilities: NodeCapabilities,
    pub multiaddrs: Vec<Multiaddr>,
    pub last_seen_ms: u64,
}

/// Concurrent registry shared across the event loop, auction handler,
/// and any future code that needs to enumerate live peers.
pub struct PeerRegistry {
    inner: RwLock<HashMap<PeerId, PeerEntry>>,
    ttl: Duration,
}

impl PeerRegistry {
    pub fn new(ttl: Duration) -> Self {
        Self {
            inner: RwLock::new(HashMap::new()),
            ttl,
        }
    }

    /// Insert or refresh a peer's capability snapshot. Multiaddrs are
    /// merged additively; capability fields overwrite.
    pub fn upsert(&self, peer: PeerId, capabilities: NodeCapabilities, addrs: Vec<Multiaddr>) {
        let now_ms = now_ms();
        let mut g = self.inner.write().unwrap();
        g.entry(peer)
            .and_modify(|e| {
                e.capabilities = capabilities.clone();
                for a in &addrs {
                    if !e.multiaddrs.contains(a) {
                        e.multiaddrs.push(a.clone());
                    }
                }
                e.last_seen_ms = now_ms;
            })
            .or_insert_with(|| PeerEntry {
                capabilities,
                multiaddrs: addrs,
                last_seen_ms: now_ms,
            });
    }

    /// Record additional dialable addresses for a peer (typically from
    /// `identify` events) without touching the capability snapshot.
    pub fn observe_addrs(&self, peer: PeerId, addrs: Vec<Multiaddr>) {
        let mut g = self.inner.write().unwrap();
        if let Some(e) = g.get_mut(&peer) {
            for a in addrs {
                if !e.multiaddrs.contains(&a) {
                    e.multiaddrs.push(a);
                }
            }
        }
    }

    pub fn get(&self, peer: &PeerId) -> Option<PeerEntry> {
        let g = self.inner.read().unwrap();
        g.get(peer).cloned()
    }

    pub fn snapshot(&self) -> Vec<(PeerId, PeerEntry)> {
        let g = self.inner.read().unwrap();
        g.iter().map(|(p, e)| (*p, e.clone())).collect()
    }

    pub fn len(&self) -> usize {
        self.inner.read().unwrap().len()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Drop any entry whose `last_seen_ms` is older than `now - ttl`.
    /// Call periodically from the event loop.
    pub fn evict_stale(&self) {
        let cutoff = now_ms().saturating_sub(self.ttl.as_millis() as u64);
        let mut g = self.inner.write().unwrap();
        g.retain(|_, e| e.last_seen_ms >= cutoff);
    }
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::protocol::NodePeerId;

    fn caps(name: &str) -> NodeCapabilities {
        NodeCapabilities {
            peer_id: NodePeerId(name.into()),
            models: vec!["qwen-72b".into()],
            max_concurrent_jobs: 4,
        }
    }

    #[test]
    fn upsert_then_get_roundtrip() {
        let reg = PeerRegistry::new(Duration::from_secs(60));
        let p = PeerId::random();
        reg.upsert(p, caps("A"), vec![]);
        let entry = reg.get(&p).unwrap();
        assert_eq!(entry.capabilities.peer_id.0, "A");
    }

    #[test]
    fn snapshot_returns_all_peers() {
        let reg = PeerRegistry::new(Duration::from_secs(60));
        reg.upsert(PeerId::random(), caps("A"), vec![]);
        reg.upsert(PeerId::random(), caps("B"), vec![]);
        assert_eq!(reg.snapshot().len(), 2);
    }

    #[test]
    fn observe_addrs_extends_without_dupes() {
        let reg = PeerRegistry::new(Duration::from_secs(60));
        let p = PeerId::random();
        reg.upsert(p, caps("A"), vec![]);
        let m: Multiaddr = "/ip4/127.0.0.1/tcp/9000".parse().unwrap();
        reg.observe_addrs(p, vec![m.clone(), m.clone()]);
        reg.observe_addrs(p, vec![m.clone()]);
        assert_eq!(reg.get(&p).unwrap().multiaddrs.len(), 1);
    }
}
