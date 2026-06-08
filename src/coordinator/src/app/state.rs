//! `AppState` — Arc-shared handles wired through axum's `State`
//! extractor. Carries the coordinator's enclave keypair (used to sign
//! dispatch tokens, routing receipts, and HTTP responses), the
//! marketplace mesh handle the auction publishes through, and the
//! peer registry built from gossipsub announcements.

use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use nautilus_enclave::EnclaveKeyPair;
use redis::aio::ConnectionManager as RedisConn;
use sqlx::PgPool;
use tokio::sync::RwLock;

use crate::mesh::{Mesh, NoopMesh, PeerRegistry};
use crate::onchain::RegisteredEnclave;
use crate::receipts::{InMemoryReceiptArchive, ReceiptArchive};

const DEFAULT_PEER_TTL: Duration = Duration::from_secs(600);

#[derive(Clone)]
pub struct AppState {
    inner: Arc<Inner>,
}

struct Inner {
    enclave_key:          Arc<EnclaveKeyPair>,
    mesh:                 Arc<dyn Mesh>,
    peer_registry:        Arc<PeerRegistry>,
    receipt_archive:      Arc<dyn ReceiptArchive>,
    on_chain:             Arc<RwLock<Option<RegisteredEnclave>>>,
    pg_pool:              RwLock<Option<PgPool>>,
    started_at_ms:        u64,
    /// SHA-256 fingerprint (hex) of the TLS certificate in use.
    /// Set once after the server binds; `None` in tests / plain-HTTP mode.
    tls_cert_fingerprint: RwLock<Option<String>>,
    /// Multiplexed Redis connection for rate limiting and short-lived caches.
    /// Set once after boot; `None` in tests that don't inject Redis.
    redis:                RwLock<Option<RedisConn>>,
}

impl AppState {
    pub fn new() -> Self {
        Self::with_full(
            Arc::new(EnclaveKeyPair::generate()),
            Arc::new(NoopMesh),
            Arc::new(PeerRegistry::new(DEFAULT_PEER_TTL)),
        )
    }

    pub fn with_mesh(mesh: Arc<dyn Mesh>) -> Self {
        Self::with_full(
            Arc::new(EnclaveKeyPair::generate()),
            mesh,
            Arc::new(PeerRegistry::new(DEFAULT_PEER_TTL)),
        )
    }

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
                pg_pool:              RwLock::new(None),
                started_at_ms,
                tls_cert_fingerprint: RwLock::new(None),
                redis:                RwLock::new(None),
            }),
        }
    }

    // ── Setters ──────────────────────────────────────────────────────────────

    pub async fn set_pg_pool(&self, pool: PgPool) {
        *self.inner.pg_pool.write().await = Some(pool);
    }

    pub async fn set_tls_cert_fingerprint(&self, fingerprint: String) {
        *self.inner.tls_cert_fingerprint.write().await = Some(fingerprint);
    }

    pub async fn set_redis(&self, conn: RedisConn) {
        *self.inner.redis.write().await = Some(conn);
    }

    // ── Getters ──────────────────────────────────────────────────────────────

    pub async fn pg_pool(&self) -> Option<PgPool> {
        self.inner.pg_pool.read().await.clone()
    }

    pub async fn tls_cert_fingerprint(&self) -> Option<String> {
        self.inner.tls_cert_fingerprint.read().await.clone()
    }

    /// Returns a cloned `ConnectionManager` (cheap — backed by an Arc).
    pub async fn redis(&self) -> Option<RedisConn> {
        self.inner.redis.read().await.clone()
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
