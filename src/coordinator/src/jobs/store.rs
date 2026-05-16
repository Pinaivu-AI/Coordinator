//! `PgJobStore` — wraps `PostgresStorage<DispatchJob>` and exposes the
//! coordinator-specific push, schedule, and status-update operations.

use anyhow::Result;
use apalis::prelude::Storage;
use apalis_sql::postgres::PostgresStorage;
use sqlx::PgPool;
use uuid::Uuid;

use super::dispatch_job::{DispatchJob, JobStatus};

pub struct PgJobStore {
    pub storage: PostgresStorage<DispatchJob>,
    pool: PgPool,
}

impl PgJobStore {
    /// Initialise the store: run apalis schema migration then connect
    /// our pool for the coordinator's own `dispatch_jobs` status table.
    pub async fn new(pool: PgPool) -> Result<Self> {
        PostgresStorage::setup(&pool).await?;
        let storage = PostgresStorage::new(pool.clone());
        Ok(Self { storage, pool })
    }

    /// Enqueue a job scheduled to fire at `job.deadline_ms`. Also
    /// writes our tracking row in `dispatch_jobs`.
    pub async fn push(&mut self, job: DispatchJob) -> Result<()> {
        sqlx::query(
            r#"
            INSERT INTO dispatch_jobs
                (request_id, primary_peer_id, dispatched_at_ms, deadline_ms,
                 status, escrow_handle_json)
            VALUES ($1, $2, $3, $4, $5, $6)
            ON CONFLICT (request_id) DO NOTHING
            "#,
        )
        .bind(job.request_id)
        .bind(&job.primary_peer_id.0)
        .bind(job.dispatched_at_ms as i64)
        .bind(job.deadline_ms as i64)
        .bind(JobStatus::Dispatched.as_str())
        .bind(&job.escrow_handle_json)
        .execute(&self.pool)
        .await?;

        // Schedule the apalis job to run at the deadline (unix seconds).
        let on = (job.deadline_ms / 1000) as i64;
        let req = apalis::prelude::Request::new(job);
        self.storage.schedule_request(req, on).await?;

        Ok(())
    }

    /// Mark a job as completed so the deadline watcher skips the refund.
    pub async fn mark_completed(&self, request_id: Uuid) -> Result<()> {
        sqlx::query(
            "UPDATE dispatch_jobs SET status = $1 WHERE request_id = $2",
        )
        .bind(JobStatus::Completed.as_str())
        .bind(request_id)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    /// Read the current tracking status for a job.
    pub async fn get_status(&self, request_id: Uuid) -> Result<Option<JobStatus>> {
        let row: Option<(String,)> =
            sqlx::query_as("SELECT status FROM dispatch_jobs WHERE request_id = $1")
                .bind(request_id)
                .fetch_optional(&self.pool)
                .await?;

        Ok(row.and_then(|(s,)| match s.as_str() {
            "Dispatched" => Some(JobStatus::Dispatched),
            "Acked" => Some(JobStatus::Acked),
            "Completed" => Some(JobStatus::Completed),
            "TimedOut" => Some(JobStatus::TimedOut),
            "Refunded" => Some(JobStatus::Refunded),
            _ => None,
        }))
    }
}
