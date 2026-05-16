//! Postgres-backed routing-receipt archive. Stores receipts as JSONB
//! so the full signed payload is retrievable for offline verification.

use anyhow::Result;
use async_trait::async_trait;
use sqlx::PgPool;

use crate::protocol::{RequestId, RoutingReceipt};

use super::ReceiptArchive;

pub struct PostgresReceiptArchive {
    pool: PgPool,
}

impl PostgresReceiptArchive {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }
}

#[async_trait]
impl ReceiptArchive for PostgresReceiptArchive {
    async fn put(&self, receipt: RoutingReceipt) -> Result<()> {
        let json = serde_json::to_value(&receipt)?;
        sqlx::query(
            r#"
            INSERT INTO routing_receipts (request_id, receipt_json)
            VALUES ($1, $2)
            ON CONFLICT (request_id) DO UPDATE
                SET receipt_json = EXCLUDED.receipt_json,
                    stored_at    = NOW()
            "#,
        )
        .bind(receipt.request_id)
        .bind(json)
        .execute(&self.pool)
        .await?;

        Ok(())
    }

    async fn get(&self, request_id: &RequestId) -> Result<Option<RoutingReceipt>> {
        let row: Option<(serde_json::Value,)> = sqlx::query_as(
            "SELECT receipt_json FROM routing_receipts WHERE request_id = $1",
        )
        .bind(request_id)
        .fetch_optional(&self.pool)
        .await?;

        match row {
            Some((json,)) => {
                let receipt: RoutingReceipt = serde_json::from_value(json)?;
                Ok(Some(receipt))
            }
            None => Ok(None),
        }
    }
}
