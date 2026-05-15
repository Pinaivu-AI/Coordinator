//! Apalis worker. Polls `DispatchJob` records, fires `refund_funds` on
//! deadline expiry, and runs the completion-ack handling pipeline when
//! the mesh feeds in a `CompletionAck`.
