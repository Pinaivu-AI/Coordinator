//! Swarm-driving event loop.
//!
//! Owns the libp2p `Swarm`, accepts [`MeshCommand`]s over an mpsc
//! channel, routes inbound gossipsub messages to auction bid collectors,
//! and handles inbound completion-ack requests from primary nodes.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use anyhow::Result;
use futures::StreamExt;
use libp2p::{
    gossipsub::{self, IdentTopic},
    identify, kad,
    multiaddr::Protocol,
    request_response,
    swarm::SwarmEvent,
    Multiaddr, PeerId, Swarm,
};
use tokio::sync::{mpsc, oneshot};
use tokio::time::interval;
use uuid::Uuid;

use super::behaviour::{PinaivuBehaviour, PinaivuBehaviourEvent};
use super::completion_proto::{CompletionAck, CompletionResponse};
use super::inference_proto::{InferenceDispatch, InferenceReply};
use super::peer_registry::PeerRegistry;
use super::topics::{ANNOUNCE, BIDS, INFERENCE_ANY, REPUTATION};
use crate::jobs::settlement_worker::SettlementJob;
use crate::payments;
use crate::protocol::{
    InferenceBid, InferenceRequest, NodeCapabilities, NodePeerId, Payout, RoutingReceipt,
};
use crate::receipts::ReceiptArchive;

const BID_CHANNEL_CAPACITY: usize = 64;
const EVICT_INTERVAL: Duration = Duration::from_secs(30);

/// Commands sent from a [`super::Libp2pMesh`] handle into the event loop.
pub enum MeshCommand {
    Publish {
        request: InferenceRequest,
        reply_tx: oneshot::Sender<Result<mpsc::Receiver<InferenceBid>>>,
    },
    /// Called after the auction to record which peer_id maps to which
    /// Sui payout address. Stored in `in_flight` so the completion
    /// handler can build payment rows without a DB look-up.
    SetPayoutAddresses {
        request_id: Uuid,
        addresses: HashMap<String, String>,
    },
    Dial {
        addr: Multiaddr,
        reply_tx: oneshot::Sender<Result<()>>,
    },
    ListenAddrs {
        reply_tx: oneshot::Sender<Vec<Multiaddr>>,
    },
    /// Send the actual inference job to a node over its existing
    /// outbound libp2p connection (works through NAT) and await the
    /// reply.
    DispatchInference {
        peer: PeerId,
        dispatch: InferenceDispatch,
        reply_tx: oneshot::Sender<Result<InferenceReply>>,
    },
}

/// Metadata stored per in-flight request so the completion handler can
/// build the routing receipt and compute payouts without a DB round-trip.
struct InFlightMeta {
    client_id: String,
    bid_set_hash: [u8; 32],
    /// peer_id → Sui payout address, from winning bid + helper bids.
    payout_addresses: HashMap<String, String>,
    /// Session this request belongs to. Used by the completion handler
    /// to upsert `node_session_cache` so future auctions can prefer
    /// the warm node.
    session_id: Uuid,
}

pub struct EventLoop {
    swarm: Swarm<PinaivuBehaviour>,
    cmd_rx: mpsc::Receiver<MeshCommand>,
    bid_subscribers: HashMap<Uuid, mpsc::Sender<InferenceBid>>,
    peer_registry: Arc<PeerRegistry>,
    enclave_key: Arc<nautilus_enclave::EnclaveKeyPair>,
    receipt_archive: Arc<dyn ReceiptArchive>,
    /// Postgres pool for inserting payment rows after CompletionAck.
    /// `None` when running in dev/test without Postgres.
    pg_pool: Option<sqlx::PgPool>,
    /// Channel to enqueue a `SettlementJob` after payment rows are inserted.
    /// `None` when the settlement worker is disabled (no sidecar).
    settlement_tx: Option<mpsc::Sender<SettlementJob>>,
    /// Per-request metadata keyed by request_id. Populated on publish,
    /// consumed on completion-ack.
    in_flight: HashMap<Uuid, InFlightMeta>,
    listen_addrs: Vec<Multiaddr>,
    ready_tx: Option<oneshot::Sender<()>>,
    /// Outbound `InferenceDispatch` requests awaiting a reply, keyed by
    /// the libp2p request id `send_request` returns.
    pending_inference: HashMap<request_response::OutboundRequestId, oneshot::Sender<Result<InferenceReply>>>,
}

impl EventLoop {
    pub fn new(
        swarm: Swarm<PinaivuBehaviour>,
        cmd_rx: mpsc::Receiver<MeshCommand>,
        peer_registry: Arc<PeerRegistry>,
        ready_tx: oneshot::Sender<()>,
        enclave_key: Arc<nautilus_enclave::EnclaveKeyPair>,
        receipt_archive: Arc<dyn ReceiptArchive>,
    ) -> Self {
        Self::with_pg(swarm, cmd_rx, peer_registry, ready_tx, enclave_key, receipt_archive, None)
    }

    pub fn with_pg(
        swarm: Swarm<PinaivuBehaviour>,
        cmd_rx: mpsc::Receiver<MeshCommand>,
        peer_registry: Arc<PeerRegistry>,
        ready_tx: oneshot::Sender<()>,
        enclave_key: Arc<nautilus_enclave::EnclaveKeyPair>,
        receipt_archive: Arc<dyn ReceiptArchive>,
        pg_pool: Option<sqlx::PgPool>,
    ) -> Self {
        Self::with_pg_and_settlement(swarm, cmd_rx, peer_registry, ready_tx, enclave_key, receipt_archive, pg_pool, None)
    }

    pub fn with_pg_and_settlement(
        swarm: Swarm<PinaivuBehaviour>,
        cmd_rx: mpsc::Receiver<MeshCommand>,
        peer_registry: Arc<PeerRegistry>,
        ready_tx: oneshot::Sender<()>,
        enclave_key: Arc<nautilus_enclave::EnclaveKeyPair>,
        receipt_archive: Arc<dyn ReceiptArchive>,
        pg_pool: Option<sqlx::PgPool>,
        settlement_tx: Option<mpsc::Sender<SettlementJob>>,
    ) -> Self {
        Self {
            swarm,
            cmd_rx,
            bid_subscribers: HashMap::new(),
            peer_registry,
            enclave_key,
            receipt_archive,
            pg_pool,
            settlement_tx,
            in_flight: HashMap::new(),
            listen_addrs: Vec::new(),
            ready_tx: Some(ready_tx),
            pending_inference: HashMap::new(),
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
            SwarmEvent::Behaviour(PinaivuBehaviourEvent::Completion(
                request_response::Event::Message {
                    peer,
                    message:
                        request_response::Message::Request {
                            channel, request, ..
                        },
                    ..
                },
            )) => {
                self.handle_completion_ack(peer, request, channel).await;
            }
            SwarmEvent::Behaviour(PinaivuBehaviourEvent::Inference(
                request_response::Event::Message {
                    message: request_response::Message::Response { request_id, response },
                    ..
                },
            )) => {
                if let Some(reply_tx) = self.pending_inference.remove(&request_id) {
                    let _ = reply_tx.send(Ok(response));
                }
            }
            SwarmEvent::Behaviour(PinaivuBehaviourEvent::Inference(
                request_response::Event::OutboundFailure { request_id, error, .. },
            )) => {
                if let Some(reply_tx) = self.pending_inference.remove(&request_id) {
                    let _ = reply_tx.send(Err(anyhow::anyhow!("inference outbound failure: {error}")));
                }
            }
            SwarmEvent::Behaviour(PinaivuBehaviourEvent::Identify(
                identify::Event::Received { peer_id, info, .. },
            )) => {
                let addrs = info.listen_addrs.clone();
                self.peer_registry.observe_addrs(peer_id, addrs.clone());
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

    async fn handle_completion_ack(
        &mut self,
        sender: PeerId,
        ack: CompletionAck,
        channel: request_response::ResponseChannel<CompletionResponse>,
    ) {
        tracing::info!(
            request_id = %ack.request_id,
            sender = %sender,
            "completion ack received"
        );

        // Verify the primary node's signature over the ack.
        if let Err(e) = ack.verify_primary() {
            tracing::warn!(request_id = %ack.request_id, err = %e, "completion ack primary sig invalid");
            let _ = self
                .swarm
                .behaviour_mut()
                .completion
                .send_response(channel, CompletionResponse::rejected(format!("primary signature invalid: {e}")));
            return;
        }

        // Verify every embedded proof.
        if let Err(e) = ack.verify_all_proofs() {
            tracing::warn!(request_id = %ack.request_id, err = %e, "completion ack proof invalid");
            let _ = self
                .swarm
                .behaviour_mut()
                .completion
                .send_response(channel, CompletionResponse::rejected(format!("proof invalid: {e}")));
            return;
        }

        let meta = self.in_flight.remove(&ack.request_id);
        let client_id = meta.as_ref().map(|m| m.client_id.as_str()).unwrap_or("").to_string();
        let bid_set_hash = meta.as_ref().map(|m| m.bid_set_hash).unwrap_or([0u8; 32]);
        let session_id = meta.as_ref().map(|m| m.session_id);
        let payout_addresses = meta
            .map(|m| m.payout_addresses)
            .unwrap_or_default();

        // Derive helper peer ids from any proofs beyond the first.
        let helper_peer_ids: Vec<NodePeerId> = ack
            .proofs
            .iter()
            .skip(1)
            .map(|p| p.node_peer_id.clone())
            .collect();

        let now_ms = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_millis() as u64)
            .unwrap_or(0);

        let primary_peer_id = ack
            .proofs
            .first()
            .map(|p| p.node_peer_id.clone())
            .unwrap_or_else(|| NodePeerId(sender.to_string()));

        // Compute per-node payout amounts from bid prices.
        let payout_lines = payments::compute_payouts(&ack, &payout_addresses);
        let payouts: Vec<Payout> = payout_lines
            .iter()
            .map(|l| Payout {
                sui_address: l.sui_address.clone(),
                amount_nanox: l.amount_nanox,
            })
            .collect();

        let receipt = RoutingReceipt {
            request_id: ack.request_id,
            client_id,
            primary_peer_id,
            helper_peer_ids,
            bid_set_hash,
            proof_ids: ack.proof_ids(),
            aggregated_output_hash: ack.aggregated_output_hash,
            payouts,
            timestamp_ms: now_ms,
            coordinator_pubkey: [0u8; 32],
            signature: Vec::new(),
        }
        .sign(self.enclave_key.signing_key());

        if let Err(e) = self.receipt_archive.put(receipt.clone()).await {
            tracing::error!(request_id = %ack.request_id, err = %e, "failed to store routing receipt");
            let _ = self
                .swarm
                .behaviour_mut()
                .completion
                .send_response(channel, CompletionResponse::rejected("storage error"));
            return;
        }

        tracing::info!(request_id = %ack.request_id, "routing receipt signed and stored");

        // Mark every contributing node as warm for this session so the
        // next auction can route the user back here. Upsert because
        // the same (peer, session) pair may already be warm from an
        // earlier turn — we just refresh `last_served_at` to NOW().
        if let (Some(pool), Some(sid)) = (&self.pg_pool, session_id) {
            let mut all_peers = vec![receipt.primary_peer_id.0.clone()];
            all_peers.extend(receipt.helper_peer_ids.iter().map(|p| p.0.clone()));
            for peer in &all_peers {
                if let Err(e) = sqlx::query(
                    "INSERT INTO node_session_cache
                        (node_peer_id, session_id, last_served_at, cache_tier)
                     VALUES ($1, $2, NOW(), 'gpu')
                     ON CONFLICT (node_peer_id, session_id) DO UPDATE SET
                        last_served_at = NOW(),
                        cache_tier     = 'gpu'",
                )
                .bind(peer)
                .bind(sid)
                .execute(pool)
                .await
                {
                    tracing::warn!(
                        request_id = %ack.request_id,
                        peer,
                        err = %e,
                        "node_session_cache upsert failed"
                    );
                }
            }
        }

        // Persist payment rows and enqueue a settlement job.
        if let Some(pool) = &self.pg_pool {
            if !payout_lines.is_empty() {
                if let Err(e) = payments::insert_pending(pool, ack.request_id, &payout_lines).await {
                    tracing::error!(request_id = %ack.request_id, err = %e, "failed to insert payment rows");
                } else {
                    tracing::info!(
                        request_id = %ack.request_id,
                        count = payout_lines.len(),
                        "payment rows queued"
                    );
                    if let Some(tx) = &self.settlement_tx {
                        let receipt_json = serde_json::to_string(&receipt)
                            .unwrap_or_else(|_| "{}".to_string());
                        let job = SettlementJob { request_id: ack.request_id, receipt_json };
                        if let Err(e) = tx.try_send(job) {
                            tracing::warn!(request_id = %ack.request_id, err = %e, "settlement job channel full — will retry on next poll");
                        }
                    }
                }
            }

        }

        let _ = self
            .swarm
            .behaviour_mut()
            .completion
            .send_response(channel, CompletionResponse::ok(receipt));
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
        // INFERENCE_ANY: coordinator publishes; subscribing keeps us in
        // the mesh but the messages are not actionable here.
        // REPUTATION: handled in a later slice.
    }

    async fn handle_command(&mut self, cmd: MeshCommand) {
        match cmd {
            MeshCommand::Publish { request, reply_tx } => {
                let result = self.publish_request(request);
                let _ = reply_tx.send(result);
            }
            MeshCommand::SetPayoutAddresses { request_id, addresses } => {
                if let Some(meta) = self.in_flight.get_mut(&request_id) {
                    meta.payout_addresses = addresses;
                }
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
            MeshCommand::DispatchInference { peer, dispatch, reply_tx } => {
                let out_id = self
                    .swarm
                    .behaviour_mut()
                    .inference
                    .send_request(&peer, dispatch);
                self.pending_inference.insert(out_id, reply_tx);
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

        self.in_flight.insert(
            request.request_id,
            InFlightMeta {
                client_id: String::new(),
                bid_set_hash: [0u8; 32],
                payout_addresses: HashMap::new(),
                session_id: request.session_id,
            },
        );

        let (tx, rx) = mpsc::channel(BID_CHANNEL_CAPACITY);
        self.bid_subscribers.insert(request.request_id, tx);
        Ok(rx)
    }
}

/// Topics every coordinator subscribes to on startup.
pub const SUBSCRIBED_TOPICS: &[&str] = &[BIDS, ANNOUNCE, INFERENCE_ANY, REPUTATION];
