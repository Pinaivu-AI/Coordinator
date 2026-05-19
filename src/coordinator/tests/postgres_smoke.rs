//! Integration tests that exercise the Postgres-backed receipt archive
//! and job store. Skipped automatically when `TEST_DATABASE_URL` is not
//! set so they don't block local development without a database.
//!
//! To run against a live database:
//!   TEST_DATABASE_URL=postgres://... cargo test --test postgres_smoke

use coordinator::persistence::postgres::connect;
use coordinator::protocol::{NodePeerId, RoutingReceipt};
use coordinator::receipts::{PostgresReceiptArchive, ReceiptArchive};
use nautilus_enclave::EnclaveKeyPair;
use uuid::Uuid;

fn db_url() -> Option<String> {
    std::env::var("TEST_DATABASE_URL").ok()
}

#[tokio::test]
async fn postgres_receipt_archive_roundtrip() {
    let Some(url) = db_url() else {
        eprintln!("TEST_DATABASE_URL not set — skipping postgres_smoke tests");
        return;
    };

    let pool = connect(&url).await.expect("connect to test db");
    let archive = PostgresReceiptArchive::new(pool.clone());

    let key = EnclaveKeyPair::generate();
    let request_id = Uuid::new_v4();

    let receipt = RoutingReceipt {
        request_id,
        client_id: "integration-test-client".into(),
        primary_peer_id: NodePeerId("12D3KooWPrimary".into()),
        helper_peer_ids: vec![NodePeerId("12D3KooWHelper".into())],
        bid_set_hash: [1u8; 32],
        proof_ids: vec![[2u8; 32]],
        aggregated_output_hash: [3u8; 32],
        payouts: vec![],
        timestamp_ms: 1_700_000_000_000,
        coordinator_pubkey: [0u8; 32],
        signature: vec![],
    }
    .sign(key.signing_key());

    // Store
    archive.put(receipt.clone()).await.expect("put receipt");

    // Retrieve
    let fetched = archive
        .get(&request_id)
        .await
        .expect("get receipt")
        .expect("receipt should exist");

    assert_eq!(fetched.request_id, request_id);
    assert_eq!(fetched.primary_peer_id.0, "12D3KooWPrimary");
    assert!(fetched.verify().is_ok(), "retrieved receipt must self-verify");

    // Idempotent upsert — should not error
    archive.put(receipt).await.expect("upsert should succeed");

    // Unknown request_id returns None
    let missing = archive
        .get(&Uuid::new_v4())
        .await
        .expect("get missing");
    assert!(missing.is_none());

    // Cleanup
    sqlx::query("DELETE FROM routing_receipts WHERE request_id = $1")
        .bind(request_id)
        .execute(&pool)
        .await
        .ok();
}

#[tokio::test]
async fn dispatch_job_status_transitions() {
    let Some(url) = db_url() else {
        return;
    };

    let pool = connect(&url).await.expect("connect to test db");
    let mut store = coordinator::jobs::store::PgJobStore::new(pool.clone())
        .await
        .expect("create job store");

    let request_id = Uuid::new_v4();
    let now_ms = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0);

    use coordinator::jobs::dispatch_job::{DispatchJob, JobStatus};

    let job = DispatchJob {
        request_id,
        primary_peer_id: NodePeerId("12D3KooWPrimary".into()),
        dispatched_at_ms: now_ms,
        deadline_ms: now_ms + 60_000,
        status: JobStatus::Dispatched,
        escrow_handle_json: "{}".into(),
    };

    store.push(job).await.expect("push job");

    let status = store
        .get_status(request_id)
        .await
        .expect("get status")
        .expect("status should exist");
    assert_eq!(status, JobStatus::Dispatched);

    store.mark_completed(request_id).await.expect("mark completed");

    let status = store
        .get_status(request_id)
        .await
        .expect("get status")
        .expect("status should exist");
    assert_eq!(status, JobStatus::Completed);

    // Cleanup
    sqlx::query("DELETE FROM dispatch_jobs WHERE request_id = $1")
        .bind(request_id)
        .execute(&pool)
        .await
        .ok();
}
