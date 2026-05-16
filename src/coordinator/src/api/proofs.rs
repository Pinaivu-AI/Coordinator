//! `GET /v1/proofs/{request_id}` — returns the signed routing receipt
//! and the bundle of per-node `ProofOfInference`s observed during
//! completion. Verifiable offline against the coordinator's pubkey.

use axum::{
    extract::{Path, State},
    Json,
};
use uuid::Uuid;

use crate::app::{AppError, AppState};
use crate::protocol::RoutingReceipt;

pub async fn get_proof(
    State(state): State<AppState>,
    Path(request_id): Path<Uuid>,
) -> Result<Json<RoutingReceipt>, AppError> {
    let receipt = state
        .receipt_archive()
        .get(&request_id)
        .await
        .map_err(|e| AppError::Internal(format!("archive lookup: {e}")))?
        .ok_or(AppError::NotFound)?;

    Ok(Json(receipt))
}
