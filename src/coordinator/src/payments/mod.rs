//! Payment ledger — inserts per-node payout rows into Postgres after
//! each verified `CompletionAck` and exposes helpers for the settlement
//! worker to drain them.

pub mod split;

use anyhow::Result;
use sqlx::PgPool;
use uuid::Uuid;

pub use split::{compute_payouts, PayoutLine};

/// Insert one `payments` row per payout line. All rows for the same
/// request land in a single `INSERT … ON CONFLICT DO NOTHING` so a
/// duplicate CompletionAck is a no-op.
pub async fn insert_pending(
    pool: &PgPool,
    request_id: Uuid,
    lines: &[PayoutLine],
) -> Result<()> {
    for line in lines {
        let id = Uuid::new_v4();
        sqlx::query(
            r#"
            INSERT INTO payments
                (id, request_id, payee_peer_id, payee_sui_address, amount_nanox, status)
            VALUES ($1, $2, $3, $4, $5, 'pending')
            ON CONFLICT DO NOTHING
            "#,
        )
        .bind(id)
        .bind(request_id)
        .bind(&line.peer_id)
        .bind(&line.sui_address)
        .bind(line.amount_nanox as i64)
        .execute(pool)
        .await?;
    }
    Ok(())
}

/// Fetch all `pending` rows for a given request, ordered by insertion.
pub async fn pending_for_request(
    pool: &PgPool,
    request_id: Uuid,
) -> Result<Vec<PendingPayment>> {
    let rows = sqlx::query_as::<_, PendingPayment>(
        r#"SELECT id, request_id, payee_peer_id, payee_sui_address, amount_nanox
           FROM payments
           WHERE request_id = $1 AND status = 'pending'
           ORDER BY created_at"#,
    )
    .bind(request_id)
    .fetch_all(pool)
    .await?;
    Ok(rows)
}

/// Mark a row as `submitted` with the PTB digest returned by the sidecar.
pub async fn mark_submitted(pool: &PgPool, id: Uuid, tx_digest: &str) -> Result<()> {
    sqlx::query(
        "UPDATE payments SET status = 'submitted', tx_digest = $1, submitted_at = NOW()
         WHERE id = $2",
    )
    .bind(tx_digest)
    .bind(id)
    .execute(pool)
    .await?;
    Ok(())
}

/// Mark a row as `confirmed` (used when we get on-chain confirmation).
pub async fn mark_confirmed(pool: &PgPool, id: Uuid) -> Result<()> {
    sqlx::query(
        "UPDATE payments SET status = 'confirmed', confirmed_at = NOW() WHERE id = $1",
    )
    .bind(id)
    .execute(pool)
    .await?;
    Ok(())
}

/// Mark a row as `failed` (after exhausting retries).
pub async fn mark_failed(pool: &PgPool, id: Uuid) -> Result<()> {
    sqlx::query("UPDATE payments SET status = 'failed' WHERE id = $1")
        .bind(id)
        .execute(pool)
        .await?;
    Ok(())
}

#[derive(Debug, sqlx::FromRow)]
pub struct PendingPayment {
    pub id: Uuid,
    pub request_id: Uuid,
    pub payee_peer_id: String,
    pub payee_sui_address: String,
    pub amount_nanox: i64,
}
