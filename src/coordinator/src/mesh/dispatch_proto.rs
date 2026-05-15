//! libp2p request-response protocol carrying:
//!   coordinator → node : `DispatchAssignment(DispatchToken)`
//!   node → coordinator : `CompletionAck { proofs, output_hash, sig }`
//!
//! Distinct from gossipsub topics: this is a direct, addressed channel
//! used after a winner has been picked.
