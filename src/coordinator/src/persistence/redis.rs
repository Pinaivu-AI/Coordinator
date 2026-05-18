//! Redis client used for replay-nonce tracking and short-lived hot
//! caches. In prod the coordinator reaches Redis through the parent
//! host's VSOCK socat bridge (TCP port 8102 → Upstash via TLS).

use anyhow::{Context, Result};
use redis::aio::ConnectionManager;
use redis::AsyncCommands;

/// Connect to Redis and verify reachability with a `PING`. Returns a
/// multiplexed `ConnectionManager` that auto-reconnects under load.
pub async fn connect(redis_url: &str) -> Result<ConnectionManager> {
    let client = redis::Client::open(redis_url).context("redis::Client::open")?;
    let mut manager = ConnectionManager::new(client)
        .await
        .context("redis ConnectionManager::new")?;
    let pong: String = redis::cmd("PING")
        .query_async(&mut manager)
        .await
        .context("redis PING")?;
    anyhow::ensure!(pong == "PONG", "unexpected PING reply: {pong}");
    Ok(manager)
}

/// Record a request-id replay-prevention nonce with TTL. Returns
/// `false` if the nonce was already present (i.e. replay attempt).
pub async fn check_and_set_nonce(
    conn: &mut ConnectionManager,
    request_id: uuid::Uuid,
    ttl_secs: u64,
) -> Result<bool> {
    let key = format!("nonce:{request_id}");
    let inserted: bool = conn.set_nx(&key, 1u8).await?;
    if inserted {
        let _: () = conn.expire(&key, ttl_secs as i64).await?;
    }
    Ok(inserted)
}
