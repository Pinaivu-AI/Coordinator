//! Liveness + attestation endpoints.
//!
//!   `GET /health`            — process liveness
//!   `GET /metrics`           — Prometheus exposition
//!   `GET /enclave_health`    — coordinator pubkey + uptime
//!   `GET /get_attestation`   — NSM document binding pubkey to PCRs
