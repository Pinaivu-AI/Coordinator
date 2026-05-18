//! Coordinator configuration loaded from environment.
//!
//! Inside the enclave these are populated via the VSOCK:7000 config
//! push at boot; locally they come from a `.env` file or the shell.
//! All persistence URLs are required — the coordinator refuses to
//! start without durable storage.

use anyhow::Context;

#[derive(Debug, Clone)]
pub struct Config {
    /// TCP socket the axum router binds. Default `127.0.0.1:4000`.
    pub bind_addr: String,
    /// libp2p multiaddr the mesh listens on. Default `/ip4/0.0.0.0/tcp/0`.
    pub libp2p_listen: String,
    /// Postgres URL — receipts, dispatch jobs, payments. In prod the
    /// VSOCK:8101 socat bridge forwards this to Supabase.
    pub database_url: String,
    /// Redis URL — replay nonces and short-lived hot caches. In prod
    /// the VSOCK:8102 socat bridge forwards this to Upstash.
    pub redis_url: String,
}

impl Config {
    pub fn from_env() -> anyhow::Result<Self> {
        let bind_addr = std::env::var("PINAIVU_BIND")
            .unwrap_or_else(|_| "127.0.0.1:4000".to_string());
        let libp2p_listen = std::env::var("PINAIVU_LIBP2P_LISTEN")
            .unwrap_or_else(|_| "/ip4/0.0.0.0/tcp/0".to_string());
        let database_url = std::env::var("DATABASE_URL")
            .context("DATABASE_URL not set (required for receipts + jobs storage)")?;
        let redis_url = std::env::var("REDIS_URL")
            .context("REDIS_URL not set (required for replay-nonce caching)")?;
        Ok(Self {
            bind_addr,
            libp2p_listen,
            database_url,
            redis_url,
        })
    }
}
