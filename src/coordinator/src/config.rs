//! Coordinator configuration loaded from environment + optional TOML.
//!
//! Scaffold — fields will be filled in as the wiring lands.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct Config {
    // TODO: bind_addr, vsock_port, postgres_url, redis_url,
    // bootstrap_peers, model_registry, settlement, …
}

impl Config {
    pub fn from_env() -> anyhow::Result<Self> {
        // TODO: dotenvy::dotenv().ok(); read env vars.
        Ok(Self::default())
    }
}
