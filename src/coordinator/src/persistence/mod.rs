//! Durable state lives outside the enclave (no persistent disk
//! available inside Nitro). All reads/writes traverse the parent's
//! VSOCK socat bridge to Postgres or Redis.

pub mod postgres;
pub mod redis;
