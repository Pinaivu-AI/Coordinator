//! Apalis-backed inference-job tracking. One job per accepted request,
//! stored in Postgres. Worker transitions: `Dispatched → Acked →
//! Completed` on success, or `Dispatched → TimedOut → Refunded` on
//! deadline expiry.

pub mod dispatch_job;
pub mod settlement_worker;
pub mod store;
pub mod worker;
