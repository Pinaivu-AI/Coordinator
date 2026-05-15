//! Composed `libp2p::NetworkBehaviour` — gossipsub for the marketplace
//! topics, Kademlia for peer routing, identify + ping + autonat + mdns
//! for connectivity, plus a request-response protocol for dispatch
//! and completion-ack messages exchanged with the primary node.
