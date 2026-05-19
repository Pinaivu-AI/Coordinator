//! Postgres connection pool and schema migrations.
//!
//! In production the coordinator reaches Postgres through the parent
//! host's VSOCK socat bridge (TCP port 8101 → Postgres). The URL is
//! injected at startup via PINAIVU_DATABASE_URL.

use anyhow::Result;
use sqlx::PgPool;

/// Connect to Postgres and ensure all coordinator tables exist.
pub async fn connect(database_url: &str) -> Result<PgPool> {
    let pool = PgPool::connect(database_url).await?;
    run_migrations(&pool).await?;
    Ok(pool)
}

/// Run coordinator DDL migrations inline. Idempotent (`IF NOT EXISTS`).
async fn run_migrations(pool: &PgPool) -> Result<()> {
    sqlx::query(
        r#"
        CREATE TABLE IF NOT EXISTS routing_receipts (
            request_id   UUID PRIMARY KEY,
            receipt_json JSONB        NOT NULL,
            stored_at    TIMESTAMPTZ  NOT NULL DEFAULT NOW()
        );

        CREATE TABLE IF NOT EXISTS dispatch_jobs (
            request_id           UUID    PRIMARY KEY,
            primary_peer_id      TEXT    NOT NULL,
            dispatched_at_ms     BIGINT  NOT NULL,
            deadline_ms          BIGINT  NOT NULL,
            status               TEXT    NOT NULL DEFAULT 'Dispatched',
            escrow_handle_json   TEXT    NOT NULL DEFAULT '{}'
        );

        CREATE TABLE IF NOT EXISTS payments (
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
        );

        CREATE INDEX IF NOT EXISTS payments_status_idx
            ON payments (status) WHERE status = 'pending';

        CREATE INDEX IF NOT EXISTS payments_request_idx
            ON payments (request_id);
        "#,
    )
    .execute(pool)
    .await?;

    Ok(())
}
