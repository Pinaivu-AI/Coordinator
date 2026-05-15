//! No-op settlement adapter. All methods succeed without transferring
//! value. Always present as a last-resort fallback so the protocol
//! works on an air-gapped network.

use super::{EscrowHandle, EscrowParams, SettlementAdapter};
use crate::proof::ProofOfInference;

pub struct FreeSettlement;

#[async_trait::async_trait]
impl SettlementAdapter for FreeSettlement {
    fn id(&self) -> &'static str {
        "free"
    }

    async fn lock_funds(&self, params: EscrowParams) -> anyhow::Result<EscrowHandle> {
        Ok(EscrowHandle {
            settlement_id: self.id().into(),
            request_id: params.request_id,
            amount_nanox: params.amount_nanox,
            chain_tx_id: None,
            payload_json: "{}".into(),
        })
    }

    async fn release_funds(
        &self,
        _handle: &EscrowHandle,
        _proof: &ProofOfInference,
    ) -> anyhow::Result<()> {
        Ok(())
    }

    async fn refund_funds(&self, _handle: &EscrowHandle) -> anyhow::Result<()> {
        Ok(())
    }
}
