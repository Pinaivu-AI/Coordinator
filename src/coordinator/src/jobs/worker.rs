//! Apalis worker for `DispatchJob`. Fires at `deadline_ms` and
//! triggers a refund via the settlement adapter when the primary node
//! did not complete the request in time.

use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use apalis::prelude::{Data, Error, Monitor, WorkerBuilder, WorkerFactoryFn};
use sqlx::PgPool;

use super::dispatch_job::DispatchJob;
use super::store::PgJobStore;
use crate::settlement::{EscrowHandle, SettlementAdapter};

/// Apalis job handler. Called at `deadline_ms` for every dispatched
/// request. Checks whether the job already completed; if not, fires
/// the settlement adapter's refund path.
pub async fn handle_dispatch_timeout(
    job: DispatchJob,
    pool: Data<PgPool>,
    settlement: Data<Arc<dyn SettlementAdapter>>,
) -> Result<(), Error> {
    let now_ms = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0);

    if now_ms < job.deadline_ms {
        tracing::debug!(request_id = %job.request_id, "deadline not yet reached");
        return Ok(());
    }

    let status_row: Option<(String,)> =
        sqlx::query_as("SELECT status FROM dispatch_jobs WHERE request_id = $1")
            .bind(job.request_id)
            .fetch_optional(&*pool)
            .await
            .map_err(|e| Error::SourceError(Arc::new(Box::new(e))))?;

    let already_done = status_row
        .map(|(s,)| matches!(s.as_str(), "Completed" | "Refunded"))
        .unwrap_or(false);

    if already_done {
        tracing::info!(request_id = %job.request_id, "job already settled; skipping");
        return Ok(());
    }

    tracing::warn!(
        request_id = %job.request_id,
        primary = %job.primary_peer_id.0,
        "dispatch deadline elapsed — firing refund"
    );

    let handle: EscrowHandle = serde_json::from_str(&job.escrow_handle_json)
        .unwrap_or_else(|_| EscrowHandle {
            settlement_id: "free".into(),
            request_id: job.request_id,
            amount_nanox: crate::protocol::NanoX(0),
            chain_tx_id: None,
            payload_json: "{}".into(),
        });

    settlement
        .refund_funds(&handle)
        .await
        .map_err(|e| {
            let io = std::io::Error::new(std::io::ErrorKind::Other, e.to_string());
            Error::SourceError(Arc::new(Box::new(io)))
        })?;

    sqlx::query("UPDATE dispatch_jobs SET status = 'Refunded' WHERE request_id = $1")
        .bind(job.request_id)
        .execute(&*pool)
        .await
        .map_err(|e| Error::SourceError(Arc::new(Box::new(e))))?;

    Ok(())
}

/// Spawn the deadline-watcher monitor on a background tokio task.
pub fn spawn_dispatch_worker(
    store: &PgJobStore,
    pool: PgPool,
    settlement: Arc<dyn SettlementAdapter>,
) -> tokio::task::JoinHandle<()> {
    let storage = store.storage.clone();

    tokio::spawn(async move {
        let worker = WorkerBuilder::new("dispatch-timeout-worker")
            .data(pool)
            .data(settlement)
            .backend(storage)
            .build_fn(handle_dispatch_timeout);

        if let Err(e) = Monitor::new().register(worker).run().await {
            tracing::error!(err = %e, "dispatch worker exited with error");
        }
    })
}
