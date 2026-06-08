//! API key management endpoints.
//!
//!   POST   /v1/keys         — create a new key under an account
//!   GET    /v1/keys         — list active keys for an account
//!   DELETE /v1/keys/:id     — revoke a key
//!
//! All three endpoints require the `x-sidecar-secret` header (the same
//! admin secret used by the existing admin endpoints). The dashboard
//! calls these server-side; end-users never send the admin secret.

use axum::{
    extract::{Path, State},
    http::HeaderMap,
    Json,
};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::app::{AppError, AppState};
use crate::auth::key::{generate_api_key, hash_api_key};

fn verify_admin(headers: &HeaderMap, state: &AppState) -> Result<(), AppError> {
    let secret = std::env::var("SIDECAR_SECRET").unwrap_or_default();
    if secret.is_empty() {
        return Err(AppError::Internal("SIDECAR_SECRET not configured".into()));
    }
    let provided = headers
        .get("x-sidecar-secret")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");
    if provided != secret {
        tracing::warn!("admin secret mismatch on /v1/keys");
        return Err(AppError::Unauthorized);
    }
    let _ = state; // state available for future use (e.g. DB-backed secrets)
    Ok(())
}

// ── Create ────────────────────────────────────────────────────────────────────

#[derive(Deserialize)]
pub struct CreateKeyRequest {
    pub account_id: Uuid,
    pub name:       Option<String>,
    pub rpm_limit:  Option<i32>,
    pub daily_limit: Option<i32>,
}

#[derive(Serialize)]
pub struct CreateKeyResponse {
    pub id:         Uuid,
    pub key:        String,   // raw key — shown once, never stored
    pub key_prefix: String,
    pub name:       Option<String>,
    pub rpm_limit:  i32,
    pub daily_limit: i32,
}

pub async fn create_key(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(req): Json<CreateKeyRequest>,
) -> Result<Json<CreateKeyResponse>, AppError> {
    verify_admin(&headers, &state)?;

    let pool = state
        .pg_pool()
        .await
        .ok_or_else(|| AppError::Internal("database not available".into()))?;

    let (raw_key, key_hash, key_prefix) = generate_api_key();
    let rpm     = req.rpm_limit.unwrap_or(10);
    let daily   = req.daily_limit.unwrap_or(100);
    let key_id  = Uuid::new_v4();

    sqlx::query(
        "INSERT INTO api_keys (id, account_id, key_hash, key_prefix, name, rpm_limit, daily_limit)
         VALUES ($1, $2, $3, $4, $5, $6, $7)",
    )
    .bind(key_id)
    .bind(req.account_id)
    .bind(&key_hash)
    .bind(&key_prefix)
    .bind(&req.name)
    .bind(rpm)
    .bind(daily)
    .execute(&pool)
    .await
    .map_err(|e| AppError::Internal(e.to_string()))?;

    Ok(Json(CreateKeyResponse {
        id: key_id,
        key: raw_key,
        key_prefix,
        name: req.name,
        rpm_limit: rpm,
        daily_limit: daily,
    }))
}

// ── List ──────────────────────────────────────────────────────────────────────

#[derive(Serialize)]
pub struct KeySummary {
    pub id:           Uuid,
    pub key_prefix:   String,
    pub name:         Option<String>,
    pub rpm_limit:    i32,
    pub daily_limit:  i32,
    pub created_at:   String,
    pub last_used_at: Option<String>,
}

pub async fn list_keys(
    State(state): State<AppState>,
    headers: HeaderMap,
    axum::extract::Query(params): axum::extract::Query<std::collections::HashMap<String, String>>,
) -> Result<Json<Vec<KeySummary>>, AppError> {
    verify_admin(&headers, &state)?;

    let account_id: Uuid = params
        .get("account_id")
        .and_then(|s| s.parse().ok())
        .ok_or_else(|| AppError::BadRequest("account_id query param required".into()))?;

    let pool = state
        .pg_pool()
        .await
        .ok_or_else(|| AppError::Internal("database not available".into()))?;

    #[derive(sqlx::FromRow)]
    struct Row {
        id:           Uuid,
        key_prefix:   String,
        name:         Option<String>,
        rpm_limit:    i32,
        daily_limit:  i32,
        created_at:   chrono::DateTime<chrono::Utc>,
        last_used_at: Option<chrono::DateTime<chrono::Utc>>,
    }

    let rows = sqlx::query_as::<_, Row>(
        "SELECT id, key_prefix, name, rpm_limit, daily_limit, created_at, last_used_at
           FROM api_keys
          WHERE account_id = $1 AND revoked_at IS NULL
          ORDER BY created_at DESC",
    )
    .bind(account_id)
    .fetch_all(&pool)
    .await
    .map_err(|e| AppError::Internal(e.to_string()))?;

    let keys = rows
        .into_iter()
        .map(|r| KeySummary {
            id:           r.id,
            key_prefix:   r.key_prefix,
            name:         r.name,
            rpm_limit:    r.rpm_limit,
            daily_limit:  r.daily_limit,
            created_at:   r.created_at.to_rfc3339(),
            last_used_at: r.last_used_at.map(|t| t.to_rfc3339()),
        })
        .collect();

    Ok(Json(keys))
}

// ── Revoke ────────────────────────────────────────────────────────────────────

#[derive(Serialize)]
pub struct RevokeResponse {
    pub revoked: bool,
}

pub async fn revoke_key(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(key_id): Path<Uuid>,
) -> Result<Json<RevokeResponse>, AppError> {
    verify_admin(&headers, &state)?;

    let pool = state
        .pg_pool()
        .await
        .ok_or_else(|| AppError::Internal("database not available".into()))?;

    let result = sqlx::query(
        "UPDATE api_keys SET revoked_at = NOW()
          WHERE id = $1 AND revoked_at IS NULL",
    )
    .bind(key_id)
    .execute(&pool)
    .await
    .map_err(|e| AppError::Internal(e.to_string()))?;

    Ok(Json(RevokeResponse {
        revoked: result.rows_affected() > 0,
    }))
}

// ── Create account (convenience) ─────────────────────────────────────────────

#[derive(Deserialize)]
pub struct CreateAccountRequest {
    pub email:       Option<String>,
    pub wallet_addr: Option<String>,
}

#[derive(Serialize)]
pub struct CreateAccountResponse {
    pub id:            Uuid,
    pub credits_nanox: i64,
    pub tier:          String,
}

pub async fn create_account(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(req): Json<CreateAccountRequest>,
) -> Result<Json<CreateAccountResponse>, AppError> {
    verify_admin(&headers, &state)?;

    let pool = state
        .pg_pool()
        .await
        .ok_or_else(|| AppError::Internal("database not available".into()))?;

    #[derive(sqlx::FromRow)]
    struct Row { id: Uuid, credits_nanox: i64, tier: String }

    let row = sqlx::query_as::<_, Row>(
        "INSERT INTO accounts (email, wallet_addr)
         VALUES ($1, $2)
         RETURNING id, credits_nanox, tier",
    )
    .bind(&req.email)
    .bind(&req.wallet_addr)
    .fetch_one(&pool)
    .await
    .map_err(|e| AppError::Internal(e.to_string()))?;

    Ok(Json(CreateAccountResponse {
        id:            row.id,
        credits_nanox: row.credits_nanox,
        tier:          row.tier,
    }))
}

