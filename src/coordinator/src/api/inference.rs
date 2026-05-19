//! `POST /v1/chat/completions` — OpenAI-shaped entry point.
//!
//! Runs the auction on behalf of the client, signs a [`DispatchToken`]
//! naming the winning primary node, and returns
//! `{ request_id, node_url, dispatch_token }`. The client then opens
//! its own HTTPS connection to `node_url` to receive the streamed
//! response — the coordinator is never in the response data path.
//!
//! In a future slice this handler will return a `307` redirect to the
//! primary node's URL with `X-Pinaivu-Dispatch-Token` in the header so
//! the standard OpenAI SDK works transparently. For now we return a
//! JSON body, which keeps the integration test simple.

use std::time::{SystemTime, UNIX_EPOCH};

use axum::{extract::State, Json};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::app::AppState;
use crate::app::AppError;
use crate::marketplace::auction::{collect_bids, pick_winner_for, DEFAULT_AUCTION_WINDOW};
use crate::protocol::{DispatchToken, InferenceRequest, NanoX, PrivacyLevel};

const DISPATCH_DEADLINE_MS: u64 = 60_000;
const DEFAULT_SETTLEMENT_ID: &str = "free";

#[derive(Debug, Deserialize)]
pub struct ChatMessage {
    pub role: String,
    pub content: String,
}

#[derive(Debug, Deserialize)]
pub struct ChatCompletionRequest {
    pub model: String,
    #[serde(default)]
    pub messages: Vec<ChatMessage>,
    /// Hex-encoded Ed25519 public key of the client, bound into the
    /// dispatch token so only this client can later authenticate to
    /// the chosen node.
    pub client_pubkey_hex: String,
    pub max_price_nanox: Option<u64>,
    #[serde(default)]
    pub privacy: Option<String>,
    /// Settlement IDs the client will accept, in preference order.
    /// Omitting this is equivalent to `["free"]`.
    #[serde(default)]
    pub accepted_settlements: Vec<String>,
}

#[derive(Debug, Serialize)]
pub struct ChatCompletionDispatch {
    pub request_id: Uuid,
    pub node_url: String,
    pub dispatch_token: DispatchToken,
}

pub async fn chat_completions(
    State(state): State<AppState>,
    Json(req): Json<ChatCompletionRequest>,
) -> Result<Json<ChatCompletionDispatch>, AppError> {
    let request_id = Uuid::new_v4();
    let session_id = Uuid::new_v4();

    let accepted_settlements = if req.accepted_settlements.is_empty() {
        vec!["free".to_string()]
    } else {
        req.accepted_settlements.clone()
    };

    let inference_request = InferenceRequest {
        request_id,
        session_id,
        model: req.model.clone(),
        privacy: parse_privacy(req.privacy.as_deref()),
        accepted_settlements: accepted_settlements.clone(),
    };

    let rx = state
        .mesh()
        .publish_request(&inference_request)
        .await
        .map_err(|e| {
            let msg = e.to_string();
            if msg.contains("NoPeersSubscribed") || msg.contains("InsufficientPeers") {
                AppError::NoNodes("no GPU nodes subscribed to this model topic".into())
            } else {
                AppError::Internal(format!("mesh publish: {e}"))
            }
        })?;

    let bids = collect_bids(rx, DEFAULT_AUCTION_WINDOW).await;
    let winner = pick_winner_for(&bids, &accepted_settlements)
        .ok_or_else(|| AppError::NoNodes("no bids matched the requested settlement".into()))?;

    // Record peer_id → Sui payout address for every bidder so the
    // completion handler can build payment rows without a DB round-trip.
    let payout_addresses: std::collections::HashMap<String, String> = bids
        .iter()
        .filter(|b| !b.payout_address.is_empty())
        .map(|b| (b.node_peer_id.0.clone(), b.payout_address.clone()))
        .collect();
    state
        .mesh()
        .set_payout_addresses(request_id, payout_addresses)
        .await;

    let client_pubkey = decode_hex_pubkey(&req.client_pubkey_hex)?;
    let now_ms = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0);

    let max_price = NanoX(req.max_price_nanox.unwrap_or(u64::MAX));

    let token = DispatchToken {
        request_id,
        client_pubkey,
        primary_peer_id: winner.node_peer_id.clone(),
        settlement_id: DEFAULT_SETTLEMENT_ID.to_string(),
        max_price_nanox: max_price,
        issued_at_ms: now_ms,
        deadline_ms: now_ms + DISPATCH_DEADLINE_MS,
        coordinator_pubkey: [0u8; 32],
        signature: Vec::new(),
    }
    .sign(state.enclave_key().signing_key());

    // Bids carry the winning node's HTTP endpoint directly — the node
    // advertises its own URL so the coordinator doesn't have to guess.
    let node_url = winner.http_endpoint.clone();

    Ok(Json(ChatCompletionDispatch {
        request_id,
        node_url,
        dispatch_token: token,
    }))
}

fn parse_privacy(s: Option<&str>) -> PrivacyLevel {
    match s.unwrap_or("standard").to_ascii_lowercase().as_str() {
        "private" => PrivacyLevel::Private,
        "fragmented" => PrivacyLevel::Fragmented,
        "maximum" => PrivacyLevel::Maximum,
        _ => PrivacyLevel::Standard,
    }
}

fn decode_hex_pubkey(s: &str) -> Result<[u8; 32], AppError> {
    let raw = hex::decode(s).map_err(|_| AppError::BadRequest("client_pubkey_hex must be hex".into()))?;
    raw.as_slice()
        .try_into()
        .map_err(|_| AppError::BadRequest("client_pubkey_hex must be 32 bytes".into()))
}
