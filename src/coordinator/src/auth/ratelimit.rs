//! Redis-backed per-key rate limiter.
//!
//! Uses INCR + EXPIRE to maintain a sliding 60-second request counter
//! per API key. The pattern is:
//!   1. INCR  key:rpm:{api_key_id}  → new count
//!   2. If count == 1, set EXPIRE 60  (first request in this window)
//!   3. If count > limit → reject with 429
//!
//! Fails open: if Redis is unavailable the request is allowed through
//! so that a Redis outage doesn't take down the entire API.

use anyhow::Result;
use redis::AsyncCommands;
use redis::aio::ConnectionManager;
use uuid::Uuid;

/// Returns `true` when the request is within the per-minute limit,
/// `false` when it should be rejected with HTTP 429.
pub async fn check_rpm(
    conn: &mut ConnectionManager,
    api_key_id: Uuid,
    limit: i32,
) -> Result<bool> {
    let redis_key = format!("rpm:{api_key_id}");
    let count: i64 = conn.incr(&redis_key, 1i64).await?;
    if count == 1 {
        // First request in this 60-second window — set expiry.
        let _: () = conn.expire(&redis_key, 60i64).await?;
    }
    Ok(count <= limit as i64)
}
