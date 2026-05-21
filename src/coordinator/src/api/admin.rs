//! Privileged endpoints used by the deploy host. Authenticated with the
//! shared SIDECAR_SECRET (the same secret the coordinator uses when
//! talking to the colocated sidecar). The deploy host knows this secret
//! because it pushed it into ~/.env.runtime; nobody else does.

use axum::{
    extract::State,
    http::{HeaderMap, StatusCode},
    response::IntoResponse,
    Json,
};
use serde::{Deserialize, Serialize};

use crate::{app::AppState, onchain::RegisteredEnclave};

#[derive(Debug, Deserialize)]
pub struct SetEnclaveIdReq {
    pub enclave_object_id: String,
    #[serde(default)]
    pub tx_digest: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct SetEnclaveIdResp {
    pub enclave_object_id: String,
    pub forwarded_to_sidecar: bool,
}

/// Push a freshly-registered Enclave object id into the running sidecar
/// (via loopback) and the coordinator's own /enclave_health cache.
/// Used by scripts/register-coordinator.sh on the deploy host so the
/// current sidecar can start settling without waiting for a reboot.
pub async fn set_enclave_id(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(req): Json<SetEnclaveIdReq>,
) -> impl IntoResponse {
    let expected = match std::env::var("SIDECAR_SECRET") {
        Ok(s) => s,
        Err(_) => {
            return (
                StatusCode::SERVICE_UNAVAILABLE,
                Json(serde_json::json!({"error": "SIDECAR_SECRET not configured"})),
            )
                .into_response();
        }
    };

    let supplied = headers
        .get("x-sidecar-secret")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");
    if supplied.as_bytes().len() != expected.as_bytes().len()
        || !constant_time_eq(supplied.as_bytes(), expected.as_bytes())
    {
        return (
            StatusCode::UNAUTHORIZED,
            Json(serde_json::json!({"error": "bad or missing X-Sidecar-Secret"})),
        )
            .into_response();
    }

    let id = req.enclave_object_id.trim().to_string();
    if id.is_empty() {
        return (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({"error": "enclave_object_id required"})),
        )
            .into_response();
    }

    // Update the coordinator's own cache so /enclave_health surfaces it.
    *state.on_chain().write().await = Some(RegisteredEnclave {
        tx_digest: req.tx_digest.clone().unwrap_or_default(),
        enclave_object_id: id.clone(),
    });

    // Forward to the colocated sidecar so vault::settle calls succeed.
    let sidecar_url = std::env::var("SIDECAR_URL")
        .unwrap_or_else(|_| "http://127.0.0.1:8200".to_string());
    let forwarded = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(5))
        .build()
        .ok()
        .map(|c| {
            let url = format!("{sidecar_url}/sui/set-enclave-id");
            let secret = expected.clone();
            let body = serde_json::json!({ "enclave_object_id": id });
            async move {
                c.put(&url)
                    .header("X-Sidecar-Secret", secret)
                    .json(&body)
                    .send()
                    .await
                    .ok()
                    .map(|r| r.status().is_success())
                    .unwrap_or(false)
            }
        });
    let forwarded_ok = match forwarded {
        Some(fut) => fut.await,
        None => false,
    };

    (
        StatusCode::OK,
        Json(SetEnclaveIdResp {
            enclave_object_id: id,
            forwarded_to_sidecar: forwarded_ok,
        }),
    )
        .into_response()
}

fn constant_time_eq(a: &[u8], b: &[u8]) -> bool {
    if a.len() != b.len() {
        return false;
    }
    let mut diff: u8 = 0;
    for (x, y) in a.iter().zip(b.iter()) {
        diff |= x ^ y;
    }
    diff == 0
}
