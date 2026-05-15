//! Settlement adapters — pluggable backends for moving value from
//! client to node after a verified completion. The free + signed-
//! receipt adapters ship in v1; payment-channel / Sui / EVM adapters
//! live behind cargo features.

pub mod free;
pub mod receipt;

use serde::{Deserialize, Serialize};

use crate::proof::ProofOfInference;
use crate::types::NanoX;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EscrowParams {
    pub request_id: uuid::Uuid,
    pub amount_nanox: NanoX,
    pub client_address: String,
    pub node_address: String,
    pub token_id: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EscrowHandle {
    pub settlement_id: String,
    pub request_id: uuid::Uuid,
    pub amount_nanox: NanoX,
    pub chain_tx_id: Option<String>,
    pub payload_json: String,
}

#[async_trait::async_trait]
pub trait SettlementAdapter: Send + Sync {
    fn id(&self) -> &'static str;
    async fn lock_funds(&self, params: EscrowParams) -> anyhow::Result<EscrowHandle>;
    async fn release_funds(&self, handle: &EscrowHandle, proof: &ProofOfInference)
        -> anyhow::Result<()>;
    async fn refund_funds(&self, handle: &EscrowHandle) -> anyhow::Result<()>;
}
