//! `AppState` — Arc-shared handles wired through axum's `State`
//! extractor. Carries the coordinator's enclave keypair (used to sign
//! dispatch tokens, routing receipts, and HTTP responses), the
//! marketplace mesh handle the auction publishes through, and the
//! peer registry built from gossipsub announcements.

use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use nautilus_enclave::EnclaveKeyPair;
use sqlx::PgPool;
use tokio::sync::RwLock;

use crate::mesh::{Mesh, NoopMesh, PeerRegistry};
use crate::onchain::RegisteredEnclave;
use crate::receipts::{InMemoryReceiptArchive, ReceiptArchive};

/// How long an un-refreshed peer stays in the in-enclave registry
/// before being evicted. Five times the default announce interval —
/// gives plenty of slack for a single missed broadcast.
const DEFAULT_PEER_TTL: Duration = Duration::from_secs(600);

#[derive(Clone)]
pub struct AppState {
    inner: Arc<Inner>,
}

struct Inner {
    enclave_key: Arc<EnclaveKeyPair>,
    mesh: Arc<dyn Mesh>,
    peer_registry: Arc<PeerRegistry>,
    receipt_archive: Arc<dyn ReceiptArchive>,
    on_chain: Arc<RwLock<Option<RegisteredEnclave>>>,
    pg_pool: RwLock<Option<PgPool>>,
    started_at_ms: u64,
}

impl AppState {
    /// New state with a freshly-generated keypair, a `NoopMesh`, and
    /// an empty `PeerRegistry`. Convenience for tests and dev.
    pub fn new() -> Self {
        Self::with_full(
            Arc::new(EnclaveKeyPair::generate()),
            Arc::new(NoopMesh),
            Arc::new(PeerRegistry::new(DEFAULT_PEER_TTL)),
        )
    }

    /// New state with an explicit mesh; keypair and peer registry are
    /// generated. Used by tests that inject an `InMemoryMesh`.
    pub fn with_mesh(mesh: Arc<dyn Mesh>) -> Self {
        Self::with_full(
            Arc::new(EnclaveKeyPair::generate()),
            mesh,
            Arc::new(PeerRegistry::new(DEFAULT_PEER_TTL)),
        )
    }

    /// New state with an explicit mesh and peer registry; the enclave
    /// keypair is generated. Backward-compatible with existing tests.
    pub fn with_mesh_and_registry(
        mesh: Arc<dyn Mesh>,
        peer_registry: Arc<PeerRegistry>,
    ) -> Self {
        Self::with_full(
            Arc::new(EnclaveKeyPair::generate()),
            mesh,
            peer_registry,
        )
    }

    /// Fully-explicit constructor — used by `main.rs` so the libp2p
    /// identity, the HTTP signing key, and the routing-receipt signing
    /// key are all the same `EnclaveKeyPair`. Defaults to an in-memory
    /// receipt archive; pass an explicit one via [`with_full_archive`]
    /// for prod (Postgres) or tests that share the archive with a
    /// libp2p event loop.
    pub fn with_full(
        enclave_key: Arc<EnclaveKeyPair>,
        mesh: Arc<dyn Mesh>,
        peer_registry: Arc<PeerRegistry>,
    ) -> Self {
        Self::with_full_archive(
            enclave_key,
            mesh,
            peer_registry,
            Arc::new(InMemoryReceiptArchive::new()),
        )
    }

    /// Like [`with_full`] but takes an explicit receipt archive so the
    /// HTTP layer and the mesh event loop share the same store.
    pub fn with_full_archive(
        enclave_key: Arc<EnclaveKeyPair>,
        mesh: Arc<dyn Mesh>,
        peer_registry: Arc<PeerRegistry>,
        receipt_archive: Arc<dyn ReceiptArchive>,
    ) -> Self {
        Self::with_full_archive_and_chain(
            enclave_key,
            mesh,
            peer_registry,
            receipt_archive,
            Arc::new(RwLock::new(None)),
        )
    }

    /// Full constructor used by `main.rs`; threads the on-chain
    /// registration cell through to `/enclave_health`.
    pub fn with_full_archive_and_chain(
        enclave_key: Arc<EnclaveKeyPair>,
        mesh: Arc<dyn Mesh>,
        peer_registry: Arc<PeerRegistry>,
        receipt_archive: Arc<dyn ReceiptArchive>,
        on_chain: Arc<RwLock<Option<RegisteredEnclave>>>,
    ) -> Self {
        let started_at_ms = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_millis() as u64)
            .unwrap_or(0);
        Self {
            inner: Arc::new(Inner {
                enclave_key,
                mesh,
                peer_registry,
                receipt_archive,
                on_chain,
                pg_pool: RwLock::new(None),
                started_at_ms,
            }),
        }
    }

    /// Attach a Postgres pool after construction so admin endpoints can
    /// inspect persistent state (payments, dispatch_jobs, etc).
    pub async fn set_pg_pool(&self, pool: PgPool) {
        *self.inner.pg_pool.write().await = Some(pool);
    }

    pub async fn pg_pool(&self) -> Option<PgPool> {
        self.inner.pg_pool.read().await.clone()
    }

    pub fn enclave_key(&self) -> &EnclaveKeyPair {
        &self.inner.enclave_key
    }

    pub fn enclave_key_arc(&self) -> Arc<EnclaveKeyPair> {
        self.inner.enclave_key.clone()
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

    pub fn receipt_archive(&self) -> &Arc<dyn ReceiptArchive> {
        &self.inner.receipt_archive
    }

    pub fn started_at_ms(&self) -> u64 {
        self.inner.started_at_ms
    }

    pub fn on_chain(&self) -> &Arc<RwLock<Option<RegisteredEnclave>>> {
        &self.inner.on_chain
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
