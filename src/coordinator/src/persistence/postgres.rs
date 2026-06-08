//! Postgres connection pool and schema migrations.
//!
//! In production the coordinator reaches Postgres through the parent
//! host's VSOCK socat bridge (TCP port 8101 → Postgres). The URL is
//! injected at startup via PINAIVU_DATABASE_URL.

use anyhow::Result;
use sqlx::PgPool;

/// Connect to Postgres and ensure all coordinator tables exist.
///
/// Retries for up to 30 seconds to tolerate the VSOCK socat bridge
/// needing a moment to start listening after enclave boot.
pub async fn connect(database_url: &str) -> Result<PgPool> {
    let mut last_err = anyhow::anyhow!("postgres connect: no attempts made");
    for attempt in 1..=10u32 {
        eprintln!("CHK 05.{attempt} PgPool::connect attempt");
        match PgPool::connect(database_url).await {
            Ok(pool) => {
                eprintln!("CHK 05.{attempt} pool open; running migrations");
                run_migrations(&pool).await?;
                eprintln!("CHK 05.{attempt} migrations done");
                return Ok(pool);
            }
            Err(e) => {
                eprintln!("CHK 05.{attempt} connect failed: {e}");
                tracing::warn!(attempt, error = %e, "postgres not ready, retrying in 3s");
                last_err = e.into();
                tokio::time::sleep(std::time::Duration::from_secs(3)).await;
            }
        }
    }
    Err(last_err)
}

/// Run coordinator DDL migrations inline. Idempotent (`IF NOT EXISTS`).
async fn run_migrations(pool: &PgPool) -> Result<()> {
    let statements: &[&str] = &[
        r#"CREATE TABLE IF NOT EXISTS routing_receipts (
            request_id      UUID PRIMARY KEY,
            receipt_json    JSONB        NOT NULL,
            stored_at       TIMESTAMPTZ  NOT NULL DEFAULT NOW(),
            walrus_blob_id  TEXT,
            created_at      TIMESTAMPTZ  NOT NULL DEFAULT NOW()
        )"#,
        "ALTER TABLE routing_receipts ADD COLUMN IF NOT EXISTS walrus_blob_id TEXT",
        "ALTER TABLE routing_receipts ADD COLUMN IF NOT EXISTS created_at TIMESTAMPTZ NOT NULL DEFAULT NOW()",
        r#"CREATE TABLE IF NOT EXISTS dispatch_jobs (
            request_id           UUID    PRIMARY KEY,
            primary_peer_id      TEXT    NOT NULL,
            dispatched_at_ms     BIGINT  NOT NULL,
            deadline_ms          BIGINT  NOT NULL,
            status               TEXT    NOT NULL DEFAULT 'Dispatched',
            escrow_handle_json   TEXT    NOT NULL DEFAULT '{}'
        )"#,
        r#"CREATE TABLE IF NOT EXISTS payments (
            id                  UUID        PRIMARY KEY,
            request_id          UUID        NOT NULL,
            payee_peer_id       TEXT        NOT NULL,
            payee_sui_address   TEXT        NOT NULL,
            amount_nanox        BIGINT      NOT NULL,
            status              TEXT        NOT NULL DEFAULT 'pending',
            tx_digest           TEXT,
            created_at          TIMESTAMPTZ NOT NULL DEFAULT NOW(),
            submitted_at        TIMESTAMPTZ,
            confirmed_at        TIMESTAMPTZ
        )"#,
        "CREATE INDEX IF NOT EXISTS payments_status_idx ON payments (status) WHERE status = 'pending'",
        "CREATE INDEX IF NOT EXISTS payments_request_idx ON payments (request_id)",

        // ── API platform ────────────────────────────────────────────────────
        r#"CREATE TABLE IF NOT EXISTS accounts (
            id              UUID        PRIMARY KEY DEFAULT gen_random_uuid(),
            email           TEXT        UNIQUE,
            wallet_addr     TEXT,
            credits_nanox   BIGINT      NOT NULL DEFAULT 5000000,
            tier            TEXT        NOT NULL DEFAULT 'free',
            created_at      TIMESTAMPTZ NOT NULL DEFAULT NOW()
        )"#,
        r#"CREATE TABLE IF NOT EXISTS api_keys (
            id              UUID        PRIMARY KEY DEFAULT gen_random_uuid(),
            account_id      UUID        NOT NULL REFERENCES accounts(id) ON DELETE CASCADE,
            key_hash        TEXT        NOT NULL UNIQUE,
            key_prefix      TEXT        NOT NULL,
            name            TEXT,
            rpm_limit       INTEGER     NOT NULL DEFAULT 10,
            daily_limit     INTEGER     NOT NULL DEFAULT 100,
            created_at      TIMESTAMPTZ NOT NULL DEFAULT NOW(),
            last_used_at    TIMESTAMPTZ,
            revoked_at      TIMESTAMPTZ
        )"#,
        "CREATE INDEX IF NOT EXISTS api_keys_hash_active_idx ON api_keys (key_hash) WHERE revoked_at IS NULL",
        r#"CREATE TABLE IF NOT EXISTS api_usage (
            id              UUID        PRIMARY KEY DEFAULT gen_random_uuid(),
            request_id      UUID,
            api_key_id      UUID        REFERENCES api_keys(id),
            model           TEXT        NOT NULL,
            input_tokens    INTEGER     NOT NULL DEFAULT 0,
            output_tokens   INTEGER     NOT NULL DEFAULT 0,
            cost_nanox      BIGINT      NOT NULL DEFAULT 0,
            latency_ms      INTEGER,
            created_at      TIMESTAMPTZ NOT NULL DEFAULT NOW()
        )"#,
        "CREATE INDEX IF NOT EXISTS api_usage_key_created_idx ON api_usage (api_key_id, created_at DESC)",
    ];

    for stmt in statements {
        sqlx::query(stmt).execute(pool).await?;
    }

    Ok(())
}
