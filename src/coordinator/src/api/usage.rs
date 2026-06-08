//! Usage tracking — writes to `api_usage` after dispatch and exposes
//! `GET /v1/usage` so developers can inspect their consumption.
//!
//! Token counts are 0 at dispatch time; they will be back-filled when
//! a `CompletionAck` arrives in the mesh event loop (future slice).

use axum::{
    extract::{Query, State},
    Extension, Json,
};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sqlx::PgPool;
use std::collections::HashMap;
use uuid::Uuid;

use crate::app::{AppError, AppState};
use crate::auth::key::ApiKeyContext;

// ── Write path ────────────────────────────────────────────────────────────────

/// Insert one row into `api_usage`. Fire-and-forget — spawn this in a
/// `tokio::spawn` so a DB hiccup never blocks the request.
pub async fn record_dispatch(
    pool: &PgPool,
    request_id: Uuid,
    api_key_id: Uuid,
    model: &str,
) {
    let _ = sqlx::query(
        "INSERT INTO api_usage (request_id, api_key_id, model)
         VALUES ($1, $2, $3)
         ON CONFLICT DO NOTHING",
    )
    .bind(request_id)
    .bind(api_key_id)
    .bind(model)
    .execute(pool)
    .await;
}

/// Update token counts and cost once the CompletionAck arrives.
/// Called from the mesh event loop (future slice).
pub async fn update_completion(
    pool: &PgPool,
    request_id: Uuid,
    input_tokens: i32,
    output_tokens: i32,
    cost_nanox: i64,
    latency_ms: i32,
) {
    let _ = sqlx::query(
        "UPDATE api_usage
            SET input_tokens  = $1,
                output_tokens = $2,
                cost_nanox    = $3,
                latency_ms    = $4
          WHERE request_id = $5",
    )
    .bind(input_tokens)
    .bind(output_tokens)
    .bind(cost_nanox)
    .bind(latency_ms)
    .bind(request_id)
    .execute(pool)
    .await;
}

// ── Read path ─────────────────────────────────────────────────────────────────

#[derive(Deserialize)]
pub struct UsageQuery {
    /// How many past days to include. Default 30, max 90.
    pub days: Option<i32>,
}

#[derive(Serialize)]
pub struct UsageRecord {
    pub request_id:    Option<Uuid>,
    pub model:         String,
    pub input_tokens:  i32,
    pub output_tokens: i32,
    pub cost_nanox:    i64,
    pub latency_ms:    Option<i32>,
    pub created_at:    String,
}

#[derive(Serialize)]
pub struct UsageSummary {
    pub total_requests:    i64,
    pub total_input_tokens:  i64,
    pub total_output_tokens: i64,
    pub total_cost_nanox:    i64,
    pub records:           Vec<UsageRecord>,
}

/// `GET /v1/usage` — returns recent usage for the authenticated key's account.
///
/// Requires API key auth (ApiKeyContext in extensions).
pub async fn get_usage(
    State(state): State<AppState>,
    Extension(ctx): Extension<ApiKeyContext>,
    Query(params): Query<UsageQuery>,
) -> Result<Json<UsageSummary>, AppError> {
    let pool = state
        .pg_pool()
        .await
        .ok_or_else(|| AppError::Internal("database not available".into()))?;

    let days = params.days.unwrap_or(30).min(90).max(1);

    #[derive(sqlx::FromRow)]
    struct Row {
        request_id:    Option<Uuid>,
        model:         String,
        input_tokens:  i32,
        output_tokens: i32,
        cost_nanox:    i64,
        latency_ms:    Option<i32>,
        created_at:    DateTime<Utc>,
    }

    // Fetch rows for all keys belonging to the same account.
    let rows = sqlx::query_as::<_, Row>(
        "SELECT u.request_id, u.model, u.input_tokens, u.output_tokens,
                u.cost_nanox, u.latency_ms, u.created_at
           FROM api_usage u
           JOIN api_keys  k ON k.id = u.api_key_id
          WHERE k.account_id = (
                    SELECT account_id FROM api_keys WHERE id = $1
                )
            AND u.created_at >= NOW() - ($2 || ' days')::INTERVAL
          ORDER BY u.created_at DESC
          LIMIT 500",
    )
    .bind(ctx.api_key_id)
    .bind(days.to_string())
    .fetch_all(&pool)
    .await
    .map_err(|e| AppError::Internal(e.to_string()))?;

    let total_requests      = rows.len() as i64;
    let total_input_tokens  = rows.iter().map(|r| r.input_tokens  as i64).sum();
    let total_output_tokens = rows.iter().map(|r| r.output_tokens as i64).sum();
    let total_cost_nanox    = rows.iter().map(|r| r.cost_nanox).sum();

    let records = rows
        .into_iter()
        .map(|r| UsageRecord {
            request_id:    r.request_id,
            model:         r.model,
            input_tokens:  r.input_tokens,
            output_tokens: r.output_tokens,
            cost_nanox:    r.cost_nanox,
            latency_ms:    r.latency_ms,
            created_at:    r.created_at.to_rfc3339(),
        })
        .collect();

    Ok(Json(UsageSummary {
        total_requests,
        total_input_tokens,
        total_output_tokens,
        total_cost_nanox,
        records,
    }))
}
