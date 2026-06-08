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
        // Context layer (Phase 16): chat sessions, turns, facts,
        // summaries, warm-node tracking, KV cache pointers.
        r#"CREATE TABLE IF NOT EXISTS sessions (
            session_id        UUID         PRIMARY KEY,
            user_address      TEXT         NOT NULL,
            model_id          TEXT         NOT NULL,
            title             TEXT,
            created_at        TIMESTAMPTZ  NOT NULL DEFAULT NOW(),
            last_updated      TIMESTAMPTZ  NOT NULL DEFAULT NOW(),
            turn_count        INT          NOT NULL DEFAULT 0,
            total_tokens      BIGINT       NOT NULL DEFAULT 0,
            total_cost_nanox  BIGINT       NOT NULL DEFAULT 0,
            walrus_blob_id    TEXT,
            prev_blob_id      TEXT,
            status            TEXT         NOT NULL DEFAULT 'active'
        )"#,
        r#"CREATE TABLE IF NOT EXISTS turns (
            turn_id           UUID         PRIMARY KEY,
            session_id        UUID         NOT NULL REFERENCES sessions(session_id),
            user_address      TEXT         NOT NULL,
            request_id        TEXT         NOT NULL,
            node_peer_id      TEXT,
            input_tokens      INT,
            output_tokens     INT,
            latency_ms        INT,
            cost_nanox        BIGINT,
            kv_token_hash     TEXT,
            proof_hash        TEXT,
            node_signature    TEXT,
            settlement_status TEXT         NOT NULL DEFAULT 'pending',
            sui_tx_digest     TEXT,
            created_at        TIMESTAMPTZ  NOT NULL DEFAULT NOW()
        )"#,
        r#"CREATE TABLE IF NOT EXISTS user_facts (
            fact_id           UUID         PRIMARY KEY,
            user_address      TEXT         NOT NULL,
            fact              TEXT         NOT NULL,
            confidence        DOUBLE PRECISION NOT NULL DEFAULT 1.0,
            source_session    UUID         REFERENCES sessions(session_id),
            created_at        TIMESTAMPTZ  NOT NULL DEFAULT NOW(),
            updated_at        TIMESTAMPTZ  NOT NULL DEFAULT NOW(),
            is_active         BOOLEAN      NOT NULL DEFAULT TRUE
        )"#,
        r#"CREATE TABLE IF NOT EXISTS session_summaries (
            summary_id        UUID         PRIMARY KEY,
            session_id        UUID         NOT NULL REFERENCES sessions(session_id),
            user_address      TEXT         NOT NULL,
            summary_text      TEXT         NOT NULL,
            messages_covered  INT          NOT NULL DEFAULT 0,
            token_count       INT          NOT NULL DEFAULT 0,
            created_at        TIMESTAMPTZ  NOT NULL DEFAULT NOW()
        )"#,
        r#"CREATE TABLE IF NOT EXISTS node_session_cache (
            node_peer_id      TEXT         NOT NULL,
            session_id        UUID         NOT NULL REFERENCES sessions(session_id),
            last_served_at    TIMESTAMPTZ  NOT NULL DEFAULT NOW(),
            cache_tier        TEXT         NOT NULL DEFAULT 'gpu',
            PRIMARY KEY (node_peer_id, session_id)
        )"#,
        r#"CREATE TABLE IF NOT EXISTS kv_cache_index (
            token_hash        TEXT         PRIMARY KEY,
            session_id        UUID         NOT NULL REFERENCES sessions(session_id),
            walrus_blob_id    TEXT         NOT NULL,
            node_peer_id      TEXT,
            size_bytes        BIGINT,
            created_at        TIMESTAMPTZ  NOT NULL DEFAULT NOW(),
            expires_at        TIMESTAMPTZ
        )"#,
        "CREATE INDEX IF NOT EXISTS idx_sessions_user ON sessions (user_address)",
        "CREATE INDEX IF NOT EXISTS idx_turns_session ON turns (session_id)",
        "CREATE INDEX IF NOT EXISTS idx_facts_user ON user_facts (user_address) WHERE is_active = TRUE",
        "CREATE INDEX IF NOT EXISTS idx_node_cache_session ON node_session_cache (session_id)",
        "CREATE INDEX IF NOT EXISTS idx_summaries_user ON session_summaries (user_address)",
    ];

    for stmt in statements {
        sqlx::query(stmt).execute(pool).await?;
    }

    Ok(())
}
