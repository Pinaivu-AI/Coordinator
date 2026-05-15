//! Sealed-bid auction. Publishes an `InferenceRequest` on the
//! appropriate gossipsub topic, collects bids for ~200 ms, filters by
//! settlement compatibility, ranks by composite score
//! (price / latency / reputation), and returns the winning bid.
