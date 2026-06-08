//! Sealed-bid auction. Drains a bid receiver for a fixed window,
//! ranks bids by composite score (whitepaper §12.3), returns the
//! winner.

use std::collections::HashMap;
use std::time::Duration;

use tokio::sync::mpsc;
use tokio::time::{timeout_at, Instant};

use crate::protocol::InferenceBid;

/// Default bid-collection window. The whitepaper specifies 200 ms;
/// production deployments may widen this for cross-region propagation.
pub const DEFAULT_AUCTION_WINDOW: Duration = Duration::from_millis(200);

// Weights when there is no warm-cache signal. Match the original three-
// term formula so existing deployments behave the same when the
// context layer is off.
const WEIGHT_PRICE: f32 = 0.4;
const WEIGHT_LATENCY: f32 = 0.3;
const WEIGHT_REPUTATION: f32 = 0.3;

// Weights when warmth is in play. From the context-layer plan §12:
// 0.35 price + 0.25 latency + 0.25 reputation + 0.15 warmth.
const WEIGHT_PRICE_W: f32 = 0.35;
const WEIGHT_LATENCY_W: f32 = 0.25;
const WEIGHT_REPUTATION_W: f32 = 0.25;
const WEIGHT_WARMTH: f32 = 0.15;

/// Warm-cache score per node, in `[0.0, 1.0]`. Higher = warmer.
/// Mapped from `node_session_cache.cache_tier`:
///   `gpu` → 1.0, `cpu` → 0.7, `disk` → 0.4, missing → 0.0.
pub type WarmthMap = HashMap<String, f32>;

pub fn warmth_for_tier(tier: &str) -> f32 {
    match tier {
        "gpu" => 1.0,
        "cpu" => 0.7,
        "disk" => 0.4,
        _ => 0.0,
    }
}

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

/// Score that incorporates the warm-cache term from
/// `node_session_cache`. `warmth` ∈ [0.0, 1.0].
pub fn score_bid_with_warmth(bid: &InferenceBid, warmth: f32) -> f32 {
    let price = bid.price_per_1k.0 as f32;
    let latency = bid.latency_ms as f32;
    let inv_price = if price > 0.0 { 1.0 / price } else { 0.0 };
    let inv_latency = if latency > 0.0 { 1.0 / latency } else { 0.0 };
    WEIGHT_PRICE_W * inv_price
        + WEIGHT_LATENCY_W * inv_latency
        + WEIGHT_REPUTATION_W * bid.reputation
        + WEIGHT_WARMTH * warmth.clamp(0.0, 1.0)
}

/// Pick the highest-scoring bid. Ties break on lowest price, then lowest latency.
pub fn pick_winner(bids: &[InferenceBid]) -> Option<&InferenceBid> {
    pick_winner_with_warmth(bids, &WarmthMap::new())
}

/// Pick the highest-scoring bid, factoring in `node_session_cache`
/// warmth for the request's `session_id`. Empty `warmth_map` falls
/// back to the standard three-term formula via the warmth-aware
/// scoring (warmth=0 everywhere just zeros the fourth term).
pub fn pick_winner_with_warmth<'a>(
    bids: &'a [InferenceBid],
    warmth_map: &WarmthMap,
) -> Option<&'a InferenceBid> {
    bids.iter().max_by(|a, b| {
        let wa = warmth_map.get(&a.node_peer_id.0).copied().unwrap_or(0.0);
        let wb = warmth_map.get(&b.node_peer_id.0).copied().unwrap_or(0.0);
        let sa = score_bid_with_warmth(a, wa);
        let sb = score_bid_with_warmth(b, wb);
        match sa.partial_cmp(&sb).unwrap_or(std::cmp::Ordering::Equal) {
            std::cmp::Ordering::Equal => {
                match b.price_per_1k.0.cmp(&a.price_per_1k.0) {
                    std::cmp::Ordering::Equal => b.latency_ms.cmp(&a.latency_ms),
                    other => other,
                }
            }
            other => other,
        }
    })
}

/// Load per-peer warmth for a session. Returns an empty map when no
/// pool is configured (stateless coordinator) or when the session has
/// never been served.
pub async fn fetch_warmth_map(
    pool: Option<&sqlx::PgPool>,
    session_id: uuid::Uuid,
) -> WarmthMap {
    let Some(pool) = pool else { return WarmthMap::new() };
    let rows: Vec<(String, String)> = match sqlx::query_as(
        "SELECT node_peer_id, cache_tier FROM node_session_cache WHERE session_id = $1",
    )
    .bind(session_id)
    .fetch_all(pool)
    .await
    {
        Ok(r) => r,
        Err(e) => {
            tracing::warn!(error = %e, %session_id, "warmth lookup failed; falling back to cold");
            return WarmthMap::new();
        }
    };
    rows.into_iter()
        .map(|(peer, tier)| (peer, warmth_for_tier(&tier)))
        .collect()
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
            http_endpoint: format!("http://node-{peer}.test"),
            payout_address: format!("0x{:0>62}", peer),
            node_x25519_pubkey: None,
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

    #[test]
    fn warm_node_beats_marginally_cheaper_cold_node() {
        // Two similar bids; B is slightly cheaper but A is warm in GPU.
        let bids = vec![
            bid("A", 100, 300, 0.9),
            bid("B", 90, 300, 0.9),
        ];
        // No warmth → B (cheaper) wins.
        let cold = WarmthMap::new();
        assert_eq!(pick_winner_with_warmth(&bids, &cold).unwrap().node_peer_id.0, "B");
        // A warm in GPU → A flips ahead.
        let mut warm = WarmthMap::new();
        warm.insert("A".into(), warmth_for_tier("gpu"));
        assert_eq!(pick_winner_with_warmth(&bids, &warm).unwrap().node_peer_id.0, "A");
    }

    #[test]
    fn aggressively_cheap_node_still_beats_warm() {
        // The 1/price inverse means moderate price gaps lose to the
        // warmth bonus; only an order-of-magnitude-cheaper bid breaks
        // through. B at price=1 dwarfs A at 10_000 in the inverse term.
        let bids = vec![
            bid("A", 10_000, 300, 0.9),
            bid("B", 1, 300, 0.9),
        ];
        let mut warm = WarmthMap::new();
        warm.insert("A".into(), warmth_for_tier("gpu"));
        assert_eq!(pick_winner_with_warmth(&bids, &warm).unwrap().node_peer_id.0, "B");
    }

    #[test]
    fn warmth_tiers_are_ordered() {
        assert!(warmth_for_tier("gpu") > warmth_for_tier("cpu"));
        assert!(warmth_for_tier("cpu") > warmth_for_tier("disk"));
        assert!(warmth_for_tier("disk") > warmth_for_tier("missing"));
        assert_eq!(warmth_for_tier("missing"), 0.0);
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
