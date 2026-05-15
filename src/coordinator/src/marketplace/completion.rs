//! Handles `CompletionAck` messages from primary nodes. Verifies every
//! attached `ProofOfInference`, hashes the aggregated output, signs and
//! writes a `RoutingReceipt`, transitions the apalis job, and triggers
//! settlement via the chosen adapter.
