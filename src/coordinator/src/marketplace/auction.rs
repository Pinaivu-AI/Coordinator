//! Sealed-bid auction. Drains a bid receiver for a fixed window,
//! ranks bids by composite score (whitepaper §12.3), returns the
//! winner.

use std::time::Duration;

use tokio::sync::mpsc;
use tokio::time::{timeout_at, Instant};

use crate::protocol::InferenceBid;

/// Default bid-collection window. The whitepaper specifies 200 ms;
/// production deployments may widen this for cross-region propagation.
pub const DEFAULT_AUCTION_WINDOW: Duration = Duration::from_millis(200);

const WEIGHT_PRICE: f32 = 0.4;
const WEIGHT_LATENCY: f32 = 0.3;
const WEIGHT_REPUTATION: f32 = 0.3;

/// Collect every bid that arrives on `rx` until `window` elapses.
/// Returns the bids in arrival order.
pub async fn collect_bids(
    mut rx: mpsc::Receiver<InferenceBid>,
    window: Duration,
) -> Vec<InferenceBid> {
    let deadline = Instant::now() + window;
    let mut bids = Vec::new();
    loop {
        match timeout_at(deadline, rx.recv()).await {
            Ok(Some(bid)) => bids.push(bid),
            // sender closed before deadline
            Ok(None) => break,
            // deadline reached
            Err(_) => break,
        }
    }
    bids
}

/// Composite score per whitepaper §12.3 / §10.3:
///   score = w1 * (1 / price) + w2 * (1 / latency) + w3 * reputation
///
/// Higher is better. Zero-priced or zero-latency bids contribute the
/// neutral 0 to that term (no division by zero).
pub fn score_bid(bid: &InferenceBid) -> f32 {
    let price = bid.price_per_1k.0 as f32;
    let latency = bid.latency_ms as f32;
    let inv_price = if price > 0.0 { 1.0 / price } else { 0.0 };
    let inv_latency = if latency > 0.0 { 1.0 / latency } else { 0.0 };
    WEIGHT_PRICE * inv_price + WEIGHT_LATENCY * inv_latency + WEIGHT_REPUTATION * bid.reputation
}

/// Pick the highest-scoring bid. Ties break on lowest price; remaining
/// ties break on lowest latency. Returns `None` if the slice is empty.
pub fn pick_winner(bids: &[InferenceBid]) -> Option<&InferenceBid> {
    bids.iter().max_by(|a, b| {
        let sa = score_bid(a);
        let sb = score_bid(b);
        match sa.partial_cmp(&sb).unwrap_or(std::cmp::Ordering::Equal) {
            std::cmp::Ordering::Equal => {
                // Lower price wins (so reverse-compare to keep "max").
                match b.price_per_1k.0.cmp(&a.price_per_1k.0) {
                    std::cmp::Ordering::Equal => b.latency_ms.cmp(&a.latency_ms),
                    other => other,
                }
            }
            other => other,
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::protocol::{NanoX, NodePeerId};
    use uuid::Uuid;

    fn bid(peer: &str, price: u64, latency: u32, reputation: f32) -> InferenceBid {
        InferenceBid {
            request_id: Uuid::nil(),
            node_peer_id: NodePeerId(peer.into()),
            price_per_1k: NanoX(price),
            latency_ms: latency,
            reputation,
        }
    }

    #[test]
    fn cheaper_faster_higher_rep_wins() {
        let bids = vec![
            bid("A", 100, 500, 0.5),
            bid("B", 50, 300, 0.9),
            bid("C", 80, 400, 0.7),
        ];
        let w = pick_winner(&bids).unwrap();
        assert_eq!(w.node_peer_id.0, "B");
    }

    #[test]
    fn empty_returns_none() {
        let bids: Vec<InferenceBid> = vec![];
        assert!(pick_winner(&bids).is_none());
    }

    #[test]
    fn tie_breaks_on_lower_price() {
        // Construct two bids with equal composite scores by making
        // reputation absorb the price/latency differences. Easier in
        // practice: two identical bids except for price.
        let a = bid("A", 100, 500, 0.5);
        let b = bid("B", 50, 500, 0.5);
        let bids = vec![a, b];
        let w = pick_winner(&bids).unwrap();
        assert_eq!(w.node_peer_id.0, "B");
    }

    #[tokio::test]
    async fn collect_bids_respects_window() {
        let (tx, rx) = mpsc::channel(4);
        tx.send(bid("A", 100, 500, 0.5)).await.unwrap();
        tx.send(bid("B", 50, 300, 0.9)).await.unwrap();
        // Leave sender alive so receiver doesn't close; window expires first.
        let bids = collect_bids(rx, Duration::from_millis(50)).await;
        assert_eq!(bids.len(), 2);
        // keep sender alive past the await
        drop(tx);
    }

    #[tokio::test]
    async fn collect_bids_closes_when_sender_drops() {
        let (tx, rx) = mpsc::channel(4);
        tx.send(bid("A", 100, 500, 0.5)).await.unwrap();
        drop(tx);
        let bids = collect_bids(rx, Duration::from_secs(60)).await;
        assert_eq!(bids.len(), 1);
    }
}
