//! Signed routing receipt — the post-completion audit artefact for an
//! inference job. Holders of `(receipt, coordinator_pubkey)` can verify
//! offline that the coordinator routed `request_id` to the recorded
//! peers and observed the listed proofs.

use ed25519_dalek::{Signature, Signer, SigningKey, Verifier, VerifyingKey};
use serde::{Deserialize, Serialize};

use super::types::{NodePeerId, RequestId};
use super::VerifyError;

/// One payout entry inside a routing receipt. `sui_address` is the
/// node's advertised `payout_address` from its bid; `amount_nanox` is
/// the share the coordinator computed from this node's proof. The
/// on-chain vault uses these to disburse from escrow.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Payout {
    pub sui_address: String,
    pub amount_nanox: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RoutingReceipt {
    pub request_id: RequestId,
    pub client_id: String,
    pub primary_peer_id: NodePeerId,
    pub helper_peer_ids: Vec<NodePeerId>,
    pub bid_set_hash: [u8; 32],
    pub proof_ids: Vec<[u8; 32]>,
    pub aggregated_output_hash: [u8; 32],
    /// Per-node payouts the on-chain vault should execute against the
    /// escrowed funds for `request_id`.
    pub payouts: Vec<Payout>,
    pub timestamp_ms: u64,
    pub coordinator_pubkey: [u8; 32],
    pub signature: Vec<u8>,
}

impl RoutingReceipt {
    pub fn canonical_bytes(&self) -> Vec<u8> {
        #[derive(Serialize)]
        struct Canonical<'a> {
            request_id: &'a RequestId,
            client_id: &'a String,
            primary_peer_id: &'a NodePeerId,
            helper_peer_ids: &'a Vec<NodePeerId>,
            bid_set_hash: &'a [u8; 32],
            proof_ids: &'a Vec<[u8; 32]>,
            aggregated_output_hash: &'a [u8; 32],
            payouts: &'a Vec<Payout>,
            timestamp_ms: u64,
            coordinator_pubkey: &'a [u8; 32],
        }
        let canonical = Canonical {
            request_id: &self.request_id,
            client_id: &self.client_id,
            primary_peer_id: &self.primary_peer_id,
            helper_peer_ids: &self.helper_peer_ids,
            bid_set_hash: &self.bid_set_hash,
            proof_ids: &self.proof_ids,
            aggregated_output_hash: &self.aggregated_output_hash,
            payouts: &self.payouts,
            timestamp_ms: self.timestamp_ms,
            coordinator_pubkey: &self.coordinator_pubkey,
        };
        serde_json::to_vec(&canonical)
            .expect("canonical serialisation is infallible for these field types")
    }

    /// Fill `coordinator_pubkey` and `signature` from `key` and return
    /// the signed receipt. Any existing values are overwritten.
    pub fn sign(mut self, key: &SigningKey) -> Self {
        self.coordinator_pubkey = key.verifying_key().to_bytes();
        let msg = self.canonical_bytes();
        let sig: Signature = key.sign(&msg);
        self.signature = sig.to_bytes().to_vec();
        self
    }

    /// Verify the coordinator's signature against `coordinator_pubkey`.
    pub fn verify(&self) -> Result<(), VerifyError> {
        let vk = VerifyingKey::from_bytes(&self.coordinator_pubkey)
            .map_err(|_| VerifyError::InvalidPublicKey)?;
        let sig = Signature::from_slice(&self.signature)
            .map_err(|_| VerifyError::InvalidSignatureBytes)?;
        vk.verify(&self.canonical_bytes(), &sig)
            .map_err(|_| VerifyError::SignatureMismatch)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ed25519_dalek::SigningKey;
    use rand::rngs::OsRng;

    fn sample() -> RoutingReceipt {
        RoutingReceipt {
            request_id: uuid::Uuid::nil(),
            client_id: "client-abc".into(),
            primary_peer_id: NodePeerId("12D3KooWPrimary".into()),
            helper_peer_ids: vec![NodePeerId("12D3KooWHelper".into())],
            bid_set_hash: [4u8; 32],
            proof_ids: vec![[5u8; 32], [6u8; 32]],
            aggregated_output_hash: [7u8; 32],
            payouts: vec![
                Payout { sui_address: "0xabc".into(), amount_nanox: 1_000 },
                Payout { sui_address: "0xdef".into(), amount_nanox: 500 },
            ],
            timestamp_ms: 1_700_000_010_000,
            coordinator_pubkey: [0u8; 32],
            signature: Vec::new(),
        }
    }

    #[test]
    fn sign_verify_roundtrip() {
        let key = SigningKey::generate(&mut OsRng);
        let signed = sample().sign(&key);
        assert!(signed.verify().is_ok());
    }

    #[test]
    fn tamper_on_helper_list_fails_verify() {
        let key = SigningKey::generate(&mut OsRng);
        let mut signed = sample().sign(&key);
        signed.helper_peer_ids.push(NodePeerId("12D3KooWInjected".into()));
        assert_eq!(signed.verify(), Err(VerifyError::SignatureMismatch));
    }

    #[test]
    fn tamper_on_output_hash_fails_verify() {
        let key = SigningKey::generate(&mut OsRng);
        let mut signed = sample().sign(&key);
        signed.aggregated_output_hash = [0xffu8; 32];
        assert_eq!(signed.verify(), Err(VerifyError::SignatureMismatch));
    }

    #[test]
    fn tamper_on_proof_ids_fails_verify() {
        let key = SigningKey::generate(&mut OsRng);
        let mut signed = sample().sign(&key);
        signed.proof_ids[0] = [0u8; 32];
        assert_eq!(signed.verify(), Err(VerifyError::SignatureMismatch));
    }
}
