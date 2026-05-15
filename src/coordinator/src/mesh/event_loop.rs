//! Swarm-driving event loop.
//!
//! Owns the libp2p `Swarm`, accepts [`MeshCommand`]s over an mpsc
//! channel, and routes inbound gossipsub messages back to whichever
//! auction is currently collecting bids for that request_id.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use anyhow::Result;
use futures::StreamExt;
use libp2p::{
    gossipsub::{self, IdentTopic},
    identify, kad,
    multiaddr::Protocol,
    swarm::SwarmEvent,
    Multiaddr, PeerId, Swarm,
};
use tokio::sync::{mpsc, oneshot};
use tokio::time::interval;
use uuid::Uuid;

use super::behaviour::{PinaivuBehaviour, PinaivuBehaviourEvent};
use super::peer_registry::PeerRegistry;
use super::topics::{ANNOUNCE, BIDS, INFERENCE_ANY, REPUTATION};
use crate::protocol::{InferenceBid, InferenceRequest, NodeCapabilities};

/// Capacity of the per-auction bid channel. 64 covers a realistic
/// upper bound on bids per 200 ms window for v1.
const BID_CHANNEL_CAPACITY: usize = 64;
/// How often the event loop sweeps stale peers from the registry.
const EVICT_INTERVAL: Duration = Duration::from_secs(30);

/// Commands sent from a [`super::Libp2pMesh`] handle into the event loop.
pub enum MeshCommand {
    Publish {
        request: InferenceRequest,
        reply_tx: oneshot::Sender<Result<mpsc::Receiver<InferenceBid>>>,
    },
    Dial {
        addr: Multiaddr,
        reply_tx: oneshot::Sender<Result<()>>,
    },
    ListenAddrs {
        reply_tx: oneshot::Sender<Vec<Multiaddr>>,
    },
}

pub struct EventLoop {
    swarm: Swarm<PinaivuBehaviour>,
    cmd_rx: mpsc::Receiver<MeshCommand>,
    bid_subscribers: HashMap<Uuid, mpsc::Sender<InferenceBid>>,
    peer_registry: Arc<PeerRegistry>,
    /// Listen addresses observed via `SwarmEvent::NewListenAddr`,
    /// already wrapped with the `/p2p/<peer_id>` suffix so they're
    /// dialable verbatim.
    listen_addrs: Vec<Multiaddr>,
    ready_tx: Option<oneshot::Sender<()>>,
}

impl EventLoop {
    pub fn new(
        swarm: Swarm<PinaivuBehaviour>,
        cmd_rx: mpsc::Receiver<MeshCommand>,
        peer_registry: Arc<PeerRegistry>,
        ready_tx: oneshot::Sender<()>,
    ) -> Self {
        Self {
            swarm,
            cmd_rx,
            bid_subscribers: HashMap::new(),
            peer_registry,
            listen_addrs: Vec::new(),
            ready_tx: Some(ready_tx),
        }
    }

    pub async fn run(mut self) {
        let mut evict = interval(EVICT_INTERVAL);
        loop {
            tokio::select! {
                event = self.swarm.select_next_some() => {
                    self.handle_swarm_event(event).await;
                }
                cmd = self.cmd_rx.recv() => match cmd {
                    Some(c) => self.handle_command(c).await,
                    None => {
                        tracing::info!("mesh command sender dropped — exiting event loop");
                        return;
                    }
                },
                _ = evict.tick() => {
                    self.peer_registry.evict_stale();
                }
            }
        }
    }

    async fn handle_swarm_event(&mut self, event: SwarmEvent<PinaivuBehaviourEvent>) {
        match event {
            SwarmEvent::NewListenAddr { address, .. } => {
                let dialable = address
                    .clone()
                    .with(Protocol::P2p(*self.swarm.local_peer_id()));
                tracing::info!(addr = %dialable, "mesh listening");
                self.listen_addrs.push(dialable);
                if let Some(tx) = self.ready_tx.take() {
                    let _ = tx.send(());
                }
            }
            SwarmEvent::Behaviour(PinaivuBehaviourEvent::Gossipsub(
                gossipsub::Event::Message {
                    propagation_source,
                    message,
                    ..
                },
            )) => {
                self.handle_gossip_message(propagation_source, message).await;
            }
            SwarmEvent::Behaviour(PinaivuBehaviourEvent::Identify(
                identify::Event::Received { peer_id, info, .. },
            )) => {
                let addrs = info.listen_addrs.clone();
                self.peer_registry.observe_addrs(peer_id, addrs.clone());
                // Feed observed addresses into Kademlia for routing.
                for addr in addrs {
                    self.swarm
                        .behaviour_mut()
                        .kademlia
                        .add_address(&peer_id, addr);
                }
            }
            SwarmEvent::Behaviour(PinaivuBehaviourEvent::Kademlia(
                kad::Event::RoutingUpdated {
                    peer, addresses, ..
                },
            )) => {
                self.peer_registry
                    .observe_addrs(peer, addresses.into_vec());
            }
            _ => {}
        }
    }

    async fn handle_gossip_message(&mut self, _src: PeerId, message: gossipsub::Message) {
        let topic = &message.topic;
        if topic == &IdentTopic::new(BIDS).hash() {
            if let Ok(bid) = serde_json::from_slice::<InferenceBid>(&message.data) {
                if let Some(tx) = self.bid_subscribers.get(&bid.request_id) {
                    let _ = tx.send(bid).await;
                }
            } else {
                tracing::warn!("malformed bid payload on /bids");
            }
        } else if topic == &IdentTopic::new(ANNOUNCE).hash() {
            if let Ok(caps) = serde_json::from_slice::<NodeCapabilities>(&message.data) {
                if let Some(author) = message.source {
                    self.peer_registry.upsert(author, caps, vec![]);
                }
            } else {
                tracing::warn!("malformed announce payload");
            }
        }
        // INFERENCE_ANY: coordinators publish; subscribing keeps us in
        // the mesh but the messages aren't actionable here.
        // REPUTATION: handled in a later slice.
    }

    async fn handle_command(&mut self, cmd: MeshCommand) {
        match cmd {
            MeshCommand::Publish { request, reply_tx } => {
                let result = self.publish_request(request);
                let _ = reply_tx.send(result);
            }
            MeshCommand::Dial { addr, reply_tx } => {
                let result = self
                    .swarm
                    .dial(addr)
                    .map_err(|e| anyhow::anyhow!("dial: {e}"));
                let _ = reply_tx.send(result);
            }
            MeshCommand::ListenAddrs { reply_tx } => {
                let _ = reply_tx.send(self.listen_addrs.clone());
            }
        }
    }

    fn publish_request(
        &mut self,
        request: InferenceRequest,
    ) -> Result<mpsc::Receiver<InferenceBid>> {
        let topic = IdentTopic::new(INFERENCE_ANY);
        let payload = serde_json::to_vec(&request)?;
        self.swarm
            .behaviour_mut()
            .gossipsub
            .publish(topic, payload)
            .map_err(|e| anyhow::anyhow!("gossipsub publish: {e}"))?;

        let (tx, rx) = mpsc::channel(BID_CHANNEL_CAPACITY);
        self.bid_subscribers.insert(request.request_id, tx);
        Ok(rx)
    }
}

/// Topics every coordinator subscribes to on startup. The coordinator
/// is also a publisher on `INFERENCE_ANY`, but we subscribe so it
/// stays in the topic mesh.
pub const SUBSCRIBED_TOPICS: &[&str] = &[BIDS, ANNOUNCE, INFERENCE_ANY, REPUTATION];
