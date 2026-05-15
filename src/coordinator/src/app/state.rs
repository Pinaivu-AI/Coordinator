//! `AppState` — Arc-shared handles wired through axum's `State`
//! extractor. Carries the coordinator's enclave keypair (used to sign
//! dispatch tokens, routing receipts, and HTTP responses), the
//! marketplace mesh handle the auction publishes through, and the
//! peer registry built from gossipsub announcements.

use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use nautilus_enclave::EnclaveKeyPair;

use crate::mesh::{Mesh, NoopMesh, PeerRegistry};

/// How long an un-refreshed peer stays in the in-enclave registry
/// before being evicted. Five times the default announce interval —
/// gives plenty of slack for a single missed broadcast.
const DEFAULT_PEER_TTL: Duration = Duration::from_secs(600);

#[derive(Clone)]
pub struct AppState {
    inner: Arc<Inner>,
}

struct Inner {
    enclave_key: EnclaveKeyPair,
    mesh: Arc<dyn Mesh>,
    peer_registry: Arc<PeerRegistry>,
    started_at_ms: u64,
}

impl AppState {
    /// New state with a `NoopMesh` and an empty `PeerRegistry`.
    /// Useful for `main.rs` before the libp2p task is spawned and
    /// for tests that don't need a marketplace.
    pub fn new() -> Self {
        Self::with_mesh_and_registry(
            Arc::new(NoopMesh),
            Arc::new(PeerRegistry::new(DEFAULT_PEER_TTL)),
        )
    }

    /// New state with an explicit mesh; the peer registry defaults to
    /// an empty one. Existing tests use this with an `InMemoryMesh`.
    pub fn with_mesh(mesh: Arc<dyn Mesh>) -> Self {
        Self::with_mesh_and_registry(mesh, Arc::new(PeerRegistry::new(DEFAULT_PEER_TTL)))
    }

    /// New state with an explicit mesh and peer registry — used by
    /// `main.rs` when wiring `Libp2pMesh`, since the event loop and
    /// the auction need to share the same registry.
    pub fn with_mesh_and_registry(
        mesh: Arc<dyn Mesh>,
        peer_registry: Arc<PeerRegistry>,
    ) -> Self {
        let started_at_ms = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_millis() as u64)
            .unwrap_or(0);
        Self {
            inner: Arc::new(Inner {
                enclave_key: EnclaveKeyPair::generate(),
                mesh,
                peer_registry,
                started_at_ms,
            }),
        }
    }

    pub fn enclave_key(&self) -> &EnclaveKeyPair {
        &self.inner.enclave_key
    }

    pub fn enclave_pubkey_bytes(&self) -> [u8; 32] {
        self.inner.enclave_key.public_key_bytes()
    }

    pub fn mesh(&self) -> &Arc<dyn Mesh> {
        &self.inner.mesh
    }

    pub fn peer_registry(&self) -> &Arc<PeerRegistry> {
        &self.inner.peer_registry
    }

    pub fn started_at_ms(&self) -> u64 {
        self.inner.started_at_ms
    }

    pub fn uptime_ms(&self) -> u64 {
        let now_ms = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_millis() as u64)
            .unwrap_or(0);
        now_ms.saturating_sub(self.inner.started_at_ms)
    }
}

impl Default for AppState {
    fn default() -> Self {
        Self::new()
    }
}
