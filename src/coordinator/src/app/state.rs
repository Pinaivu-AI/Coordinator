//! `AppState` — Arc-shared handles wired through axum's `State`
//! extractor. Carries the coordinator's enclave keypair (used to sign
//! dispatch tokens, routing receipts, and HTTP responses) plus the
//! marketplace mesh handle the auction publishes through. More
//! service handles land here as slices come online.

use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use nautilus_enclave::EnclaveKeyPair;

use crate::mesh::{Mesh, NoopMesh};

#[derive(Clone)]
pub struct AppState {
    inner: Arc<Inner>,
}

struct Inner {
    enclave_key: EnclaveKeyPair,
    mesh: Arc<dyn Mesh>,
    started_at_ms: u64,
}

impl AppState {
    /// New state with a `NoopMesh`. Suitable for the dev `main.rs`
    /// until the real libp2p mesh lands; auctions will time out with
    /// zero bids and return `NotFound`.
    pub fn new() -> Self {
        Self::with_mesh(Arc::new(NoopMesh))
    }

    /// New state with an explicit mesh — used by tests to inject an
    /// `InMemoryMesh` carrying pre-seeded bids.
    pub fn with_mesh(mesh: Arc<dyn Mesh>) -> Self {
        let started_at_ms = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_millis() as u64)
            .unwrap_or(0);
        Self {
            inner: Arc::new(Inner {
                enclave_key: EnclaveKeyPair::generate(),
                mesh,
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
