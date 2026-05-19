//! HTTP client for the in-enclave Sui sidecar.
//!
//! The Rust coordinator never holds the operator private key. It asks
//! the colocated TypeScript sidecar (`scripts/sidecar-server.ts`) to
//! build, sign, and submit Sui transactions on its behalf. Communicates
//! over loopback HTTP authenticated with a shared `SIDECAR_SECRET`
//! produced by the enclave init at boot.

use std::sync::Arc;
use std::time::Duration;

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use tokio::sync::RwLock;

/// What the sidecar knows about the on-chain enclave object after a
/// successful registration. Surfaced on `GET /enclave_health`.
#[derive(Debug, Clone, Default, Serialize)]
pub struct RegisteredEnclave {
    pub tx_digest: String,
    pub enclave_object_id: String,
}

pub struct SidecarClient {
    base_url: String,
    secret: String,
    http: reqwest::Client,
}

impl SidecarClient {
    pub fn from_env() -> Result<Self> {
        let base_url = std::env::var("SIDECAR_URL")
            .unwrap_or_else(|_| "http://127.0.0.1:8200".to_string());
        let secret = std::env::var("SIDECAR_SECRET")
            .context("SIDECAR_SECRET not set — init should populate this")?;
        let http = reqwest::Client::builder()
            .timeout(Duration::from_secs(60))
            .build()
            .context("build reqwest client")?;
        Ok(Self {
            base_url,
            secret,
            http,
        })
    }

    /// Poll `/health` until it responds 200 or we hit the deadline.
    pub async fn wait_ready(&self, max_attempts: u32) -> Result<()> {
        for attempt in 1..=max_attempts {
            let url = format!("{}/health", self.base_url);
            if let Ok(resp) = self.http.get(&url).send().await {
                if resp.status().is_success() {
                    return Ok(());
                }
            }
            tokio::time::sleep(Duration::from_millis(500)).await;
            if attempt == max_attempts {
                anyhow::bail!("sidecar did not become ready after {max_attempts} attempts");
            }
        }
        Ok(())
    }

    /// POST `/sui/register-enclave` with a base64-encoded NSM
    /// attestation document. Sidecar returns the freshly-minted
    /// `Enclave<ENCLAVE>` shared object id + tx digest.
    pub async fn register_enclave(
        &self,
        attestation_b64: &str,
    ) -> Result<RegisteredEnclave> {
        let url = format!("{}/sui/register-enclave", self.base_url);
        let resp = self
            .http
            .post(&url)
            .header("X-Sidecar-Secret", &self.secret)
            .json(&RegisterEnclaveReq {
                attestation_b64: attestation_b64.to_string(),
            })
            .send()
            .await
            .with_context(|| format!("POST {url}"))?;

        let status = resp.status();
        let body = resp.text().await.unwrap_or_default();
        if !status.is_success() {
            anyhow::bail!("sidecar returned {status}: {body}");
        }
        let parsed: RegisterEnclaveResp =
            serde_json::from_str(&body).with_context(|| format!("decode body: {body}"))?;
        Ok(RegisteredEnclave {
            tx_digest: parsed.tx_digest,
            enclave_object_id: parsed
                .enclave_object_id
                .unwrap_or_else(|| "<missing>".to_string()),
        })
    }

    /// POST `/sui/settle` — ask the sidecar to call `vault::settle` for
    /// one `(request_id, payee, amount)` triple. The sidecar builds the
    /// PTB, attaches the signed receipt, and submits it.
    /// Returns the Sui transaction digest.
    pub async fn settle(
        &self,
        request_id: &str,
        payee_sui_address: &str,
        amount_nanox: u64,
        receipt_b64: &str,
    ) -> Result<String> {
        let url = format!("{}/sui/settle", self.base_url);
        let resp = self
            .http
            .post(&url)
            .header("X-Sidecar-Secret", &self.secret)
            .json(&SettleReq {
                request_id: request_id.to_string(),
                payee_sui_address: payee_sui_address.to_string(),
                amount_nanox,
                receipt_b64: receipt_b64.to_string(),
            })
            .send()
            .await
            .with_context(|| format!("POST {url}"))?;

        let status = resp.status();
        let body = resp.text().await.unwrap_or_default();
        if !status.is_success() {
            anyhow::bail!("sidecar settle returned {status}: {body}");
        }
        let parsed: SettleResp =
            serde_json::from_str(&body).with_context(|| format!("decode settle body: {body}"))?;
        Ok(parsed.tx_digest)
    }
}

#[derive(Serialize)]
struct RegisterEnclaveReq {
    attestation_b64: String,
}

#[derive(Deserialize)]
struct RegisterEnclaveResp {
    tx_digest: String,
    enclave_object_id: Option<String>,
}

#[derive(Serialize)]
struct SettleReq {
    request_id: String,
    payee_sui_address: String,
    amount_nanox: u64,
    receipt_b64: String,
}

#[derive(Deserialize)]
struct SettleResp {
    tx_digest: String,
}

/// Spawn a background task that registers this enclave on-chain (via
/// the sidecar) and retries until it succeeds. The `state` cell is
/// updated once registration lands so `/enclave_health` can surface
/// the on-chain object id.
pub fn spawn_registration(
    sidecar: SidecarClient,
    attestation_b64: String,
    state: Arc<RwLock<Option<RegisteredEnclave>>>,
) {
    tokio::spawn(async move {
        // First wait for the sidecar process to come up.
        if let Err(e) = sidecar.wait_ready(60).await {
            tracing::error!(?e, "sidecar never became ready");
            return;
        }
        tracing::info!("sidecar /health ok, registering enclave on-chain");

        let mut backoff = Duration::from_secs(2);
        loop {
            match sidecar.register_enclave(&attestation_b64).await {
                Ok(reg) => {
                    tracing::info!(
                        tx_digest = %reg.tx_digest,
                        enclave_object_id = %reg.enclave_object_id,
                        "enclave registered on Sui"
                    );
                    *state.write().await = Some(reg);
                    return;
                }
                Err(e) => {
                    tracing::warn!(?e, "register-enclave failed, retrying after {:?}", backoff);
                    tokio::time::sleep(backoff).await;
                    backoff = (backoff * 2).min(Duration::from_secs(60));
                }
            }
        }
    });
}
