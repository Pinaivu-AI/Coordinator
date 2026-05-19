//! Compute per-node payout amounts from a `CompletionAck`.
//!
//! v1 rule: each node receives exactly the `price_paid_nanox` it declared
//! in its own `ProofOfInference`. The primary node takes the first proof;
//! helpers take subsequent proofs. If a proof's price is zero the node
//! still gets a row (for auditability) but will receive nothing on-chain.

use pinaivu_protocol::mesh::completion_proto::CompletionAck;

#[derive(Debug, Clone)]
pub struct PayoutLine {
    pub peer_id: String,
    pub sui_address: String,
    pub amount_nanox: u64,
}

/// Derive payout lines from a verified `CompletionAck`.
/// `payout_addresses` maps peer_id → Sui address and comes from the
/// winning bids the auction recorded.
pub fn compute_payouts(
    ack: &CompletionAck,
    payout_addresses: &std::collections::HashMap<String, String>,
) -> Vec<PayoutLine> {
    ack.proofs
        .iter()
        .filter_map(|proof| {
            let peer_id = proof.node_peer_id.0.clone();
            let sui_address = payout_addresses.get(&peer_id)?.clone();
            Some(PayoutLine {
                peer_id,
                sui_address,
                amount_nanox: proof.price_paid_nanox.0,
            })
        })
        .collect()
}
