//! Composed libp2p network behaviour for the coordinator.
//!
//! Layered for v1:
//!   - `gossipsub` for marketplace broadcast topics (requests, bids,
//!     announces, reputation roots).
//!   - `kademlia` for peer routing — bootstrap + peer_id → multiaddr
//!     lookups when a bidder isn't already in our local cache.
//!   - `identify` for protocol negotiation + observed-address learning.
//!   - `ping` for liveness and RTT observation.
//!   - `request_response::cbor` for the direct completion-ack channel:
//!     node_1 → CompletionAck → coordinator → CompletionResponse.

use std::time::Duration;

use anyhow::{Context, Result};
use libp2p::{
    gossipsub::{self, MessageAuthenticity, ValidationMode},
    identify, identity,
    kad::{self, store::MemoryStore},
    ping, request_response,
    swarm::NetworkBehaviour,
    PeerId, StreamProtocol,
};

use super::completion_proto::{CompletionAck, CompletionResponse, COMPLETION_PROTOCOL};

/// Application-level protocol version reported by `identify`.
pub const PROTOCOL_VERSION: &str = "/pinaivu/coordinator/1.0.0";

/// Stream protocol identifier used by Kademlia. Keeping a Pinaivu
/// prefix isolates our DHT from the public libp2p DHT.
pub const KAD_PROTOCOL: StreamProtocol = StreamProtocol::new("/pinaivu/kad/1.0.0");

#[derive(NetworkBehaviour)]
pub struct PinaivuBehaviour {
    pub gossipsub: gossipsub::Behaviour,
    pub kademlia: kad::Behaviour<MemoryStore>,
    pub identify: identify::Behaviour,
    pub ping: ping::Behaviour,
    pub completion: request_response::cbor::Behaviour<CompletionAck, CompletionResponse>,
}

impl PinaivuBehaviour {
    /// Construct the behaviour, deriving each sub-behaviour from the
    /// node's libp2p identity. Returns an error if gossipsub config
    /// validation fails (it can if heartbeat parameters disagree).
    pub fn new(key: &identity::Keypair) -> Result<Self> {
        let local_peer_id = PeerId::from(key.public());

        let gossipsub_config = gossipsub::ConfigBuilder::default()
            .heartbeat_interval(Duration::from_secs(1))
            .validation_mode(ValidationMode::Strict)
            // Cap message size to keep DoS surface small; bid + request
            // payloads are well under 64 KiB.
            .max_transmit_size(64 * 1024)
            .build()
            .map_err(|e| anyhow::anyhow!("gossipsub config: {e}"))?;

        let gossipsub = gossipsub::Behaviour::new(
            MessageAuthenticity::Signed(key.clone()),
            gossipsub_config,
        )
        .map_err(|e| anyhow::anyhow!("gossipsub init: {e}"))?;

        let mut kad_config = kad::Config::new(KAD_PROTOCOL);
        kad_config.set_query_timeout(Duration::from_secs(30));
        let kademlia = kad::Behaviour::with_config(
            local_peer_id,
            MemoryStore::new(local_peer_id),
            kad_config,
        );

        let identify = identify::Behaviour::new(
            identify::Config::new(PROTOCOL_VERSION.into(), key.public())
                .with_agent_version(format!("pinaivu-coordinator/{}", env!("CARGO_PKG_VERSION"))),
        );

        let ping = ping::Behaviour::new(
            ping::Config::new().with_interval(Duration::from_secs(15)),
        );

        let completion = request_response::cbor::Behaviour::new(
            [(COMPLETION_PROTOCOL, request_response::ProtocolSupport::Full)],
            request_response::Config::default(),
        );

        Ok(Self {
            gossipsub,
            kademlia,
            identify,
            ping,
            completion,
        })
    }
}

/// Derive a libp2p identity keypair from raw 32-byte Ed25519 secret
/// bytes. Used so the coordinator's libp2p PeerId is the same Ed25519
/// identity bound into the NSM attestation — the network address and
/// the cryptographic identity are the same object.
pub fn libp2p_identity_from_ed25519_secret(secret: &[u8; 32]) -> Result<identity::Keypair> {
    let mut buf = *secret;
    identity::Keypair::ed25519_from_bytes(&mut buf)
        .context("decode ed25519 secret bytes into libp2p keypair")
}
