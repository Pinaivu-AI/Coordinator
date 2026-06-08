//! Coordinator configuration loaded from environment.

use anyhow::Context;

#[derive(Debug, Clone)]
pub struct Config {
    /// TCP socket the axum router binds. Default `127.0.0.1:4000`.
    pub bind_addr: String,
    /// libp2p multiaddr the mesh listens on. Default `/ip4/0.0.0.0/tcp/0`.
    pub libp2p_listen: String,
    pub database_url: String,
    pub redis_url: String,
    /// PEM-encoded TLS certificate. If both `tls_cert_pem` and
    /// `tls_key_pem` are set the server binds HTTPS; otherwise a
    /// self-signed certificate is generated at boot inside the enclave.
    pub tls_cert_pem: Option<String>,
    /// PEM-encoded TLS private key matching `tls_cert_pem`.
    pub tls_key_pem: Option<String>,
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
        let tls_cert_pem = std::env::var("PINAIVU_TLS_CERT").ok();
        let tls_key_pem  = std::env::var("PINAIVU_TLS_KEY").ok();
        Ok(Self {
            bind_addr,
            libp2p_listen,
            database_url,
            redis_url,
            tls_cert_pem,
            tls_key_pem,
        })
    }

    /// Returns true when an operator-supplied TLS certificate and key are present.
    pub fn has_tls_certs(&self) -> bool {
        self.tls_cert_pem.is_some() && self.tls_key_pem.is_some()
    }
}
