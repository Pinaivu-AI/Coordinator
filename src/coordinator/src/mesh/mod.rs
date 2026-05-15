//! libp2p mesh — the coordinator's view of the public peer-to-peer
//! marketplace network. Owns the swarm, exposes channels for publishing
//! inference requests and receiving bids / capability announcements /
//! reputation updates / completion acks.

pub mod behaviour;
pub mod dispatch_proto;
pub mod event_loop;
pub mod topics;
