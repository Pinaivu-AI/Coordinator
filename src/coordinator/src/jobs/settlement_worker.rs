//! Apalis worker that drains `pending` payment rows and submits
//! `vault::settle` PTBs via the in-enclave sidecar.
//!
//! Each `SettlementJob` carries one `request_id`. The worker loads all
//! `pending` rows for that request, calls `SidecarClient::settle` once
//! per payee (one PTB per row — keeps gas simple), and marks each row
//! `submitted`. Rows that fail after the worker retries are marked
//! `failed`; the vault's `refund` path is available to the client as
//! a last resort.

use std::sync::Arc;

use apalis::prelude::{Data, Error, Monitor, WorkerBuilder, WorkerFactoryFn};
use serde::{Deserialize, Serialize};
use sqlx::PgPool;
use uuid::Uuid;

use crate::onchain::SidecarClient;
use crate::payments;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SettlementJob {
    pub request_id: Uuid,
    /// BCS+base64 encoded signed routing receipt — pre-computed by the
    /// completion handler so this worker doesn't need the enclave key.
    pub receipt_b64: String,
}

pub async fn handle_settlement(
    job: SettlementJob,
    pool: Data<PgPool>,
    sidecar: Data<Arc<SidecarClient>>,
) -> Result<(), Error> {
    let rows = payments::pending_for_request(&pool, job.request_id)
        .await
        .map_err(anyhow_err)?;

    if rows.is_empty() {
        tracing::debug!(request_id = %job.request_id, "no pending payments");
        return Ok(());
    }

    for row in rows {
        match sidecar
            .settle(
                &job.request_id.to_string(),
                &row.payee_sui_address,
                row.amount_nanox as u64,
                &job.receipt_b64,
            )
            .await
        {
            Ok(digest) => {
                tracing::info!(
                    request_id = %job.request_id,
                    payee = %row.payee_sui_address,
                    amount_nanox = row.amount_nanox,
                    tx_digest = %digest,
                    "payment submitted"
                );
                payments::mark_submitted(&pool, row.id, &digest)
                    .await
                    .map_err(anyhow_err)?;
            }
            Err(e) => {
                tracing::error!(
                    request_id = %job.request_id,
                    payee = %row.payee_sui_address,
                    err = %e,
                    "settle call failed — marking payment failed"
                );
                payments::mark_failed(&pool, row.id).await.map_err(anyhow_err)?;
            }
        }
    }

    Ok(())
}

/// Spawn the settlement worker monitor alongside the dispatch-timeout worker.
pub fn spawn_settlement_worker(
    pool: PgPool,
    sidecar: Arc<SidecarClient>,
    storage: apalis_sql::postgres::PostgresStorage<SettlementJob>,
) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        let worker = WorkerBuilder::new("settlement-worker")
            .data(pool)
            .data(sidecar)
            .backend(storage)
            .build_fn(handle_settlement);

        if let Err(e) = Monitor::new().register(worker).run().await {
            tracing::error!(err = %e, "settlement worker exited with error");
        }
    })
}

fn anyhow_err(e: anyhow::Error) -> Error {
    let io = std::io::Error::new(std::io::ErrorKind::Other, e.to_string());
    Error::SourceError(Arc::new(Box::new(io)))
}
