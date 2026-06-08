//! API key authentication middleware for axum.
//!
//! Attach with `.route_layer(middleware::from_fn_with_state(state, require_api_key))`
//! on any Router whose routes should be protected.
//!
//! Flow:
//!   1. Extract `Authorization: Bearer sk-pnv-...` header
//!   2. SHA-256 hash the key, look it up in Postgres
//!   3. Check per-minute rate limit in Redis (fail-open on Redis error)
//!   4. Fire-and-forget `last_used_at` update
//!   5. Inject `ApiKeyContext` into request extensions for downstream handlers

pub mod key;
pub mod ratelimit;

use axum::{
    extract::{Request, State},
    middleware::Next,
    response::Response,
};

use crate::app::{AppError, AppState};
use key::ApiKeyContext;

pub async fn require_api_key(
    State(state): State<AppState>,
    mut req: Request,
    next: Next,
) -> Result<Response, AppError> {
    // ── 1. Extract Bearer token ──────────────────────────────────────────────
    let raw_key = req
        .headers()
        .get("Authorization")
        .and_then(|v| v.to_str().ok())
        .and_then(|s| s.strip_prefix("Bearer "))
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .ok_or(AppError::Unauthorized)?
        .to_string();

    // ── 2. Look up in Postgres ───────────────────────────────────────────────
    let pool = state
        .pg_pool()
        .await
        .ok_or_else(|| AppError::Internal("database not available".into()))?;

    let hash = key::hash_api_key(&raw_key);
    let ctx: ApiKeyContext = key::lookup_key(&pool, &hash)
        .await
        .map_err(|e| AppError::Internal(e.to_string()))?
        .ok_or(AppError::Unauthorized)?;

    // ── 3. Rate limit (Redis, fail-open) ────────────────────────────────────
    if let Some(mut redis) = state.redis().await {
        match ratelimit::check_rpm(&mut redis, ctx.api_key_id, ctx.rpm_limit).await {
            Ok(false) => return Err(AppError::RateLimited),
            Ok(true) | Err(_) => {}
        }
    }

    // ── 4. Touch last_used_at (fire-and-forget) ──────────────────────────────
    {
        let pool2 = pool.clone();
        let key_id = ctx.api_key_id;
        tokio::spawn(async move { key::touch_key(&pool2, key_id).await });
    }

    // ── 5. Inject context for downstream handlers ───────────────────────────
    req.extensions_mut().insert(ctx);
    Ok(next.run(req).await)
}
