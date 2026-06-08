//! `GET /v1/models` — returns all models currently available from
//! online nodes with per-token pricing.
//!
//! Public endpoint — no API key required. Clients use this to
//! discover what to put in the `model` field of their requests.

use axum::{extract::State, Json};
use serde::Serialize;

use crate::app::AppState;

#[derive(Serialize)]
pub struct ModelPricing {
    pub input_per_1m_tokens_nanox:  u64,
    pub output_per_1m_tokens_nanox: u64,
}

#[derive(Serialize)]
pub struct ModelObject {
    pub id:              String,
    pub object:          &'static str,
    pub owned_by:        &'static str,
    pub nodes_available: usize,
    pub pricing:         ModelPricing,
}

#[derive(Serialize)]
pub struct ModelList {
    pub object: &'static str,
    pub data:   Vec<ModelObject>,
}

/// Hardcoded baseline pricing per model (NanoX per 1M tokens).
/// These will be replaced by on-chain model config in a later phase.
fn baseline_pricing(model_id: &str) -> ModelPricing {
    match model_id {
        id if id.contains("72b") || id.contains("72B") => ModelPricing {
            input_per_1m_tokens_nanox:  50_000,
            output_per_1m_tokens_nanox: 150_000,
        },
        id if id.contains("32b") || id.contains("32B") => ModelPricing {
            input_per_1m_tokens_nanox:  20_000,
            output_per_1m_tokens_nanox: 60_000,
        },
        _ => ModelPricing {
            input_per_1m_tokens_nanox:  5_000,
            output_per_1m_tokens_nanox: 15_000,
        },
    }
}

pub async fn list_models(State(state): State<AppState>) -> Json<ModelList> {
    // Aggregate unique model IDs from the live peer registry.
    let snapshot = state.peer_registry().snapshot();

    // Count nodes per model.
    let mut model_counts: std::collections::HashMap<String, usize> =
        std::collections::HashMap::new();
    for (_, entry) in &snapshot {
        for model in &entry.capabilities.models {
            *model_counts.entry(model.clone()).or_insert(0) += 1;
        }
    }

    let mut data: Vec<ModelObject> = model_counts
        .into_iter()
        .map(|(id, count)| ModelObject {
            pricing:         baseline_pricing(&id),
            object:          "model",
            owned_by:        "pinaivu",
            nodes_available: count,
            id,
        })
        .collect();

    // Stable sort by id for deterministic responses.
    data.sort_by(|a, b| a.id.cmp(&b.id));

    Json(ModelList { object: "list", data })
}
