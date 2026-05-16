//! `GET /v1/nodes` — current snapshot of the peer registry: peer id,
//! advertised capabilities, latest cached reputation score.

use axum::{extract::State, Json};
use serde::Serialize;

use crate::app::AppState;

#[derive(Serialize)]
pub struct NodeSnapshot {
    pub peer_id: String,
    pub models: Vec<String>,
    pub max_concurrent_jobs: u32,
    pub multiaddrs: Vec<String>,
    pub last_seen_ms: u64,
}

pub async fn list_nodes(State(state): State<AppState>) -> Json<Vec<NodeSnapshot>> {
    let nodes = state
        .peer_registry()
        .snapshot()
        .into_iter()
        .map(|(peer_id, entry)| NodeSnapshot {
            peer_id: peer_id.to_string(),
            models: entry.capabilities.models,
            max_concurrent_jobs: entry.capabilities.max_concurrent_jobs,
            multiaddrs: entry.multiaddrs.iter().map(|m| m.to_string()).collect(),
            last_seen_ms: entry.last_seen_ms,
        })
        .collect();
    Json(nodes)
}
