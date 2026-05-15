//! Signed-receipt adapter — node does the work first and trusts the
//! client to pay against a signed `ProofOfInference`. No on-chain
//! escrow. Suitable for micro-payments or trusted clusters.

use super::{EscrowHandle, EscrowParams, SettlementAdapter};
use crate::protocol::ProofOfInference;

pub struct ReceiptSettlement;

#[async_trait::async_trait]
impl SettlementAdapter for ReceiptSettlement {
    fn id(&self) -> &'static str {
        "receipt"
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
        // TODO: persist proof to ledger for later collection.
        Ok(())
    }

    async fn refund_funds(&self, _handle: &EscrowHandle) -> anyhow::Result<()> {
        Ok(())
    }
}
