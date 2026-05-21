//! Privileged endpoints used by the deploy host. Authenticated with the
//! shared SIDECAR_SECRET (the same secret the coordinator uses when
//! talking to the colocated sidecar). The deploy host knows this secret
//! because it pushed it into ~/.env.runtime; nobody else does.

use axum::{
    extract::{Path, State},
    http::{HeaderMap, StatusCode},
    response::IntoResponse,
    Json,
};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::{app::AppState, onchain::RegisteredEnclave};

fn check_secret(headers: &HeaderMap) -> Result<String, (StatusCode, Json<serde_json::Value>)> {
    let raw = std::env::var("SIDECAR_SECRET").map_err(|_| {
        (
            StatusCode::SERVICE_UNAVAILABLE,
            Json(serde_json::json!({"error": "SIDECAR_SECRET not configured"})),
        )
    })?;
    let expected = raw.trim();
    let supplied = headers
        .get("x-sidecar-secret")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("")
        .trim();
    if supplied.as_bytes().len() != expected.as_bytes().len()
        || !constant_time_eq(supplied.as_bytes(), expected.as_bytes())
    {
        tracing::warn!(
            supplied_len = supplied.len(),
            expected_len = expected.len(),
            raw_env_len = raw.len(),
            "admin secret mismatch"
        );
        return Err((
            StatusCode::UNAUTHORIZED,
            Json(serde_json::json!({"error": "bad or missing X-Sidecar-Secret"})),
        ));
    }
    Ok(expected.to_string())
}

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
    let expected = match check_secret(&headers) {
        Ok(s) => s,
        Err((code, body)) => return (code, body).into_response(),
    };

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

#[derive(Debug, Serialize)]
pub struct PaymentRow {
    pub id: String,
    pub request_id: String,
    pub payee_peer_id: String,
    pub payee_sui_address: String,
    pub amount_nanox: i64,
    pub status: String,
    pub tx_digest: Option<String>,
    pub created_at: String,
    pub submitted_at: Option<String>,
    pub confirmed_at: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct SettlementStatusResp {
    pub request_id: String,
    pub payments: Vec<PaymentRow>,
}

/// Inspect all payment rows for a request_id. Auth'd; used to debug why
/// vault::settle hasn't landed on-chain when the routing receipt exists.
pub async fn settlement_status(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(request_id): Path<String>,
) -> impl IntoResponse {
    if let Err((code, body)) = check_secret(&headers) {
        return (code, body).into_response();
    }

    let req_uuid = match Uuid::parse_str(&request_id) {
        Ok(u) => u,
        Err(_) => {
            return (
                StatusCode::BAD_REQUEST,
                Json(serde_json::json!({"error": "request_id is not a uuid"})),
            )
                .into_response();
        }
    };

    let pool = match state.pg_pool().await {
        Some(p) => p,
        None => {
            return (
                StatusCode::SERVICE_UNAVAILABLE,
                Json(serde_json::json!({"error": "pg pool not attached"})),
            )
                .into_response();
        }
    };

    let rows = sqlx::query_as::<_, (Uuid, Uuid, String, String, i64, String, Option<String>, chrono::DateTime<chrono::Utc>, Option<chrono::DateTime<chrono::Utc>>, Option<chrono::DateTime<chrono::Utc>>)>(
        "SELECT id, request_id, payee_peer_id, payee_sui_address, amount_nanox, status, tx_digest, created_at, submitted_at, confirmed_at
         FROM payments
         WHERE request_id = $1
         ORDER BY created_at"
    )
        .bind(req_uuid)
        .fetch_all(&pool)
        .await;

    match rows {
        Ok(rows) => {
            let payments = rows
                .into_iter()
                .map(|r| PaymentRow {
                    id: r.0.to_string(),
                    request_id: r.1.to_string(),
                    payee_peer_id: r.2,
                    payee_sui_address: r.3,
                    amount_nanox: r.4,
                    status: r.5,
                    tx_digest: r.6,
                    created_at: r.7.to_rfc3339(),
                    submitted_at: r.8.map(|t| t.to_rfc3339()),
                    confirmed_at: r.9.map(|t| t.to_rfc3339()),
                })
                .collect();
            (
                StatusCode::OK,
                Json(SettlementStatusResp {
                    request_id,
                    payments,
                }),
            )
                .into_response()
        }
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({"error": format!("query payments: {e}")})),
        )
            .into_response(),
    }
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
