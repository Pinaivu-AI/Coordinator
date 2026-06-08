//! API key generation, hashing, and Postgres lookup.
//!
//! Raw keys are shown to the developer exactly once on creation and
//! never stored. Only the SHA-256 hash is persisted — the same model
//! used by Stripe, GitHub, and OpenAI.
//!
//! Key format: `sk-pnv-<48 alphanumeric chars>`
//! Key prefix (stored for display): first 16 characters

use rand::distributions::Alphanumeric;
use rand::Rng;
use sha2::{Digest, Sha256};
use sqlx::PgPool;
use uuid::Uuid;

/// Context injected into request extensions after a key is validated.
/// Downstream handlers read this via `req.extensions().get::<ApiKeyContext>()`.
#[derive(Debug, Clone)]
pub struct ApiKeyContext {
    pub api_key_id: Uuid,
    pub account_id: Uuid,
    pub rpm_limit:  i32,
    pub daily_limit: i32,
}

/// Generate a fresh API key. Returns `(raw_key, key_hash, key_prefix)`.
///
/// - `raw_key`    — show to the developer once, never store
/// - `key_hash`   — SHA-256 hex, store in `api_keys.key_hash`
/// - `key_prefix` — first 16 chars of `raw_key`, store for dashboard display
pub fn generate_api_key() -> (String, String, String) {
    let suffix: String = rand::thread_rng()
        .sample_iter(&Alphanumeric)
        .take(48)
        .map(char::from)
        .collect();
    let raw    = format!("sk-pnv-{suffix}");
    let hash   = hash_api_key(&raw);
    let prefix = raw[..16].to_string();
    (raw, hash, prefix)
}

/// SHA-256 hex digest of the raw key string.
pub fn hash_api_key(raw: &str) -> String {
    hex::encode(Sha256::digest(raw.as_bytes()))
}

/// Look up an active (non-revoked) key by its hash.
/// Returns `None` if the key does not exist or has been revoked.
pub async fn lookup_key(
    pool: &PgPool,
    key_hash: &str,
) -> anyhow::Result<Option<ApiKeyContext>> {
    #[derive(sqlx::FromRow)]
    struct Row {
        id:          Uuid,
        account_id:  Uuid,
        rpm_limit:   i32,
        daily_limit: i32,
    }

    let row = sqlx::query_as::<_, Row>(
        "SELECT id, account_id, rpm_limit, daily_limit
           FROM api_keys
          WHERE key_hash = $1 AND revoked_at IS NULL",
    )
    .bind(key_hash)
    .fetch_optional(pool)
    .await?;

    Ok(row.map(|r| ApiKeyContext {
        api_key_id:  r.id,
        account_id:  r.account_id,
        rpm_limit:   r.rpm_limit,
        daily_limit: r.daily_limit,
    }))
}

/// Update `last_used_at` for a key. Called fire-and-forget — a failure
/// here must not block or fail the request.
pub async fn touch_key(pool: &PgPool, api_key_id: Uuid) {
    let _ = sqlx::query(
        "UPDATE api_keys SET last_used_at = NOW() WHERE id = $1",
    )
    .bind(api_key_id)
    .execute(pool)
    .await;
}
