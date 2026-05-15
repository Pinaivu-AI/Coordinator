//! libp2p mesh — the coordinator's view of the public peer-to-peer
//! marketplace network. Owns the swarm, exposes channels for publishing
//! inference requests and receiving bids / capability announcements /
//! reputation updates / completion acks.
//!
//! v1 ships only the [`Mesh`] trait + an in-memory test impl. The
//! real libp2p `Swarm`-driven impl lives in [`behaviour`] +
//! [`event_loop`] and lands in the next slice.

pub mod behaviour;
pub mod dispatch_proto;
pub mod event_loop;
pub mod test_mesh;
pub mod topics;

use anyhow::Result;
use async_trait::async_trait;
use tokio::sync::mpsc;

use crate::protocol::{InferenceBid, InferenceRequest};

pub use test_mesh::InMemoryMesh;

/// Abstract surface the auction needs from the marketplace network.
///
/// Real impl is libp2p gossipsub-backed; tests use [`InMemoryMesh`];
/// an idle dev coordinator uses [`NoopMesh`].
#[async_trait]
pub trait Mesh: Send + Sync {
    /// Publish an inference request to the marketplace and return a
    /// receiver that yields bids matching this request_id. The
    /// receiver closes when no more bids are coming.
    async fn publish_request(
        &self,
        request: &InferenceRequest,
    ) -> Result<mpsc::Receiver<InferenceBid>>;
}

/// A mesh that never produces bids. Default for `main.rs` until the
/// real libp2p mesh lands; every auction times out with zero bids.
pub struct NoopMesh;

#[async_trait]
impl Mesh for NoopMesh {
    async fn publish_request(
        &self,
        _request: &InferenceRequest,
    ) -> Result<mpsc::Receiver<InferenceBid>> {
        // Drop the sender immediately — the auction's `collect_bids`
        // closes as soon as the receiver sees the channel is empty.
        let (_tx, rx) = mpsc::channel(1);
        Ok(rx)
    }
}
