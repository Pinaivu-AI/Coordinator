//! libp2p mesh — the coordinator's view of the public peer-to-peer
//! marketplace network. Owns the swarm, exposes channels for publishing
//! inference requests and receiving bids / capability announcements /
//! reputation updates.

pub mod behaviour;
pub mod completion_proto;
pub mod dispatch_proto;
pub mod event_loop;
pub mod peer_registry;
pub mod test_mesh;
pub mod topics;

use std::sync::Arc;
use std::time::Duration;

use anyhow::{Context, Result};
use async_trait::async_trait;
use libp2p::{gossipsub::IdentTopic, Multiaddr};
use tokio::sync::{mpsc, oneshot};

use crate::protocol::{InferenceBid, InferenceRequest};
use crate::receipts::ReceiptArchive;

pub use completion_proto::{CompletionAck, CompletionResponse};
pub use peer_registry::{PeerEntry, PeerRegistry};
pub use test_mesh::InMemoryMesh;

use event_loop::{EventLoop, MeshCommand, SUBSCRIBED_TOPICS};

/// Abstract surface the auction needs from the marketplace network.
/// Real impl is [`Libp2pMesh`]; tests use [`InMemoryMesh`]; idle dev
/// coordinators use [`NoopMesh`].
#[async_trait]
pub trait Mesh: Send + Sync {
    async fn publish_request(
        &self,
        request: &InferenceRequest,
    ) -> Result<mpsc::Receiver<InferenceBid>>;
}

/// A mesh that never produces bids. Default for `main.rs` until the
/// real libp2p mesh is wired in via [`spawn_libp2p_mesh`].
pub struct NoopMesh;

#[async_trait]
impl Mesh for NoopMesh {
    async fn publish_request(
        &self,
        _request: &InferenceRequest,
    ) -> Result<mpsc::Receiver<InferenceBid>> {
        let (_tx, rx) = mpsc::channel(1);
        Ok(rx)
    }
}

/// Real libp2p-backed mesh. Forwards every operation as a command to
/// the [`EventLoop`] running in its own tokio task.
pub struct Libp2pMesh {
    cmd_tx: mpsc::Sender<MeshCommand>,
}

impl Libp2pMesh {
    /// Dial an explicit multiaddr (useful in tests and for bootstrap
    /// peers).
    pub async fn dial(&self, addr: Multiaddr) -> Result<()> {
        let (tx, rx) = oneshot::channel();
        self.cmd_tx
            .send(MeshCommand::Dial {
                addr,
                reply_tx: tx,
            })
            .await
            .context("send dial command")?;
        rx.await.context("dial reply")?
    }

    /// Return the swarm's currently-bound listen addresses (with the
    /// `/p2p/<peer_id>` suffix appended so they're dialable verbatim).
    pub async fn listen_addrs(&self) -> Vec<Multiaddr> {
        let (tx, rx) = oneshot::channel();
        if self
            .cmd_tx
            .send(MeshCommand::ListenAddrs { reply_tx: tx })
            .await
            .is_err()
        {
            return Vec::new();
        }
        rx.await.unwrap_or_default()
    }
}

#[async_trait]
impl Mesh for Libp2pMesh {
    async fn publish_request(
        &self,
        request: &InferenceRequest,
    ) -> Result<mpsc::Receiver<InferenceBid>> {
        let (tx, rx) = oneshot::channel();
        self.cmd_tx
            .send(MeshCommand::Publish {
                request: request.clone(),
                reply_tx: tx,
            })
            .await
            .context("send publish command")?;
        rx.await.context("publish reply")?
    }
}

/// Handle returned by [`spawn_libp2p_mesh`]. Keep it alive for the
/// duration of the process; dropping the handle aborts the event-loop
/// task.
pub struct MeshHandle {
    pub mesh: Arc<Libp2pMesh>,
    pub listen_addrs: Vec<Multiaddr>,
    pub event_loop_task: tokio::task::JoinHandle<()>,
}

/// Build the libp2p swarm bound to `listen_addr`, subscribe to the
/// marketplace topics, and spawn the [`EventLoop`] on a tokio task.
/// Resolves once the first listen address is bound so callers know
/// where to dial.
///
/// The same `Arc<EnclaveKeyPair>` is used to seed both the libp2p
/// identity (so PeerId is derived from the attested key) and any
/// future coordinator-side signing the event loop needs to do.
pub async fn spawn_libp2p_mesh(
    enclave_key: Arc<nautilus_enclave::EnclaveKeyPair>,
    listen_addr: Multiaddr,
    peer_registry: Arc<PeerRegistry>,
    receipt_archive: Arc<dyn ReceiptArchive>,
) -> Result<MeshHandle> {
    let secret = enclave_key.secret_bytes();
    let identity = behaviour::libp2p_identity_from_ed25519_secret(&secret)?;

    let mut swarm = libp2p::SwarmBuilder::with_existing_identity(identity)
        .with_tokio()
        .with_tcp(
            libp2p::tcp::Config::default(),
            libp2p::noise::Config::new,
            libp2p::yamux::Config::default,
        )
        .map_err(|e| anyhow::anyhow!("tcp transport: {e}"))?
        .with_behaviour(|key| {
            behaviour::PinaivuBehaviour::new(key)
                .map_err(|e| Box::<dyn std::error::Error + Send + Sync>::from(e.to_string()))
        })
        .map_err(|e| anyhow::anyhow!("compose behaviour: {e}"))?
        .with_swarm_config(|c| c.with_idle_connection_timeout(Duration::from_secs(60)))
        .build();

    for t in SUBSCRIBED_TOPICS {
        swarm
            .behaviour_mut()
            .gossipsub
            .subscribe(&IdentTopic::new(*t))
            .map_err(|e| anyhow::anyhow!("subscribe {t}: {e}"))?;
    }

    swarm
        .listen_on(listen_addr)
        .map_err(|e| anyhow::anyhow!("listen_on: {e}"))?;

    let (cmd_tx, cmd_rx) = mpsc::channel(64);
    let (ready_tx, ready_rx) = oneshot::channel();
    let event_loop = EventLoop::new(swarm, cmd_rx, peer_registry, ready_tx, enclave_key, receipt_archive);
    let event_loop_task = tokio::spawn(event_loop.run());

    // Wait for the first NewListenAddr — bounded so a bad config
    // surfaces fast.
    let _ = tokio::time::timeout(Duration::from_secs(5), ready_rx).await;

    let mesh = Arc::new(Libp2pMesh { cmd_tx });
    let listen_addrs = mesh.listen_addrs().await;

    Ok(MeshHandle {
        mesh,
        listen_addrs,
        event_loop_task,
    })
}
