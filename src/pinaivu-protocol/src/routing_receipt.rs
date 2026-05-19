//! Signed routing receipt — the post-completion audit artefact for an
//! inference job. The signed portion is the *settlement subset*
//! `(request_id, aggregated_output_hash, payouts)` wrapped in a BCS
//! `IntentMessage`, matching the on-chain `pinaivu::receipts::
//! ReceiptPayload` so the vault contract can verify the signature.
//! Other fields (client_id, helper_peer_ids, proof_ids, bid_set_hash)
//! are descriptive metadata the off-chain explorer renders but are
//! NOT cryptographically committed in v1.

use ed25519_dalek::{Signature, Signer, SigningKey, Verifier, VerifyingKey};
use serde::{Deserialize, Serialize};

use super::types::{NodePeerId, RequestId};
use super::VerifyError;

/// Intent scope for routing receipts. Must match the constant of the
/// same name in the on-chain `pinaivu::receipts` module so a signature
/// produced by the coordinator verifies via `enclave::verify_signature`.
pub const INTENT_ROUTING_RECEIPT: u8 = 1;

/// One payout entry inside a routing receipt. `sui_address` is the
/// node's advertised `payout_address` from its bid; `amount_nanox` is
/// the share the coordinator computed from this node's proof. The
/// on-chain vault uses these to disburse from the treasury.
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
    /// Per-node payouts the on-chain vault should execute against
    /// the treasury. Signed.
    pub payouts: Vec<Payout>,
    /// Unix-millis. Signed (part of the IntentMessage envelope).
    pub timestamp_ms: u64,
    pub coordinator_pubkey: [u8; 32],
    pub signature: Vec<u8>,
}

/// BCS-encoded payload that matches the on-chain `ReceiptPayload`
/// struct shape exactly (field order matters).
#[derive(Serialize)]
struct ReceiptPayloadBcs {
    request_id: Vec<u8>,
    aggregated_output_hash: Vec<u8>,
    payouts: Vec<PayoutBcs>,
}

/// BCS shape matching on-chain `pinaivu::receipts::Payout`. Sui
/// addresses are 32-byte fixed arrays (no length prefix on the wire).
#[derive(Serialize)]
struct PayoutBcs {
    sui_address: [u8; 32],
    amount: u64,
}

/// BCS envelope matching on-chain `IntentMessage<ReceiptPayload>`.
#[derive(Serialize)]
struct IntentMessage {
    intent: u8,
    timestamp_ms: u64,
    payload: ReceiptPayloadBcs,
}

impl RoutingReceipt {
    /// BCS-encoded IntentMessage bytes — exactly what the coordinator
    /// signs and what the on-chain `enclave::verify_signature` checks.
    pub fn intent_message_bytes(&self) -> Vec<u8> {
        let payload = ReceiptPayloadBcs {
            request_id: self.request_id.as_bytes().to_vec(),
            aggregated_output_hash: self.aggregated_output_hash.to_vec(),
            payouts: self
                .payouts
                .iter()
                .map(|p| PayoutBcs {
                    sui_address: parse_sui_address(&p.sui_address),
                    amount: p.amount_nanox,
                })
                .collect(),
        };
        let msg = IntentMessage {
            intent: INTENT_ROUTING_RECEIPT,
            timestamp_ms: self.timestamp_ms,
            payload,
        };
        bcs::to_bytes(&msg).expect("bcs encoding is infallible for these field types")
    }

    /// Fill `coordinator_pubkey` and `signature` from `key` and return
    /// the signed receipt. Any existing values are overwritten.
    pub fn sign(mut self, key: &SigningKey) -> Self {
        self.coordinator_pubkey = key.verifying_key().to_bytes();
        let msg = self.intent_message_bytes();
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
        vk.verify(&self.intent_message_bytes(), &sig)
            .map_err(|_| VerifyError::SignatureMismatch)
    }
}

/// Parse a Sui address (hex, possibly `0x`-prefixed, up to 32 bytes
/// long) into a left-zero-padded 32-byte array — matches Sui's
/// on-chain representation. Invalid input yields all-zeros so a bad
/// payout_address fails signature verification cleanly rather than
/// panicking inside `bcs::to_bytes`.
fn parse_sui_address(s: &str) -> [u8; 32] {
    let trimmed = s.trim().trim_start_matches("0x");
    let bytes = match hex::decode(trimmed) {
        Ok(b) if b.len() <= 32 => b,
        _ => return [0u8; 32],
    };
    let mut out = [0u8; 32];
    out[32 - bytes.len()..].copy_from_slice(&bytes);
    out
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
                Payout {
                    sui_address: "0x0000000000000000000000000000000000000000000000000000000000000abc".into(),
                    amount_nanox: 1_000,
                },
                Payout {
                    sui_address: "0x0000000000000000000000000000000000000000000000000000000000000def".into(),
                    amount_nanox: 500,
                },
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
    fn tamper_on_output_hash_fails_verify() {
        let key = SigningKey::generate(&mut OsRng);
        let mut signed = sample().sign(&key);
        signed.aggregated_output_hash = [0xffu8; 32];
        assert_eq!(signed.verify(), Err(VerifyError::SignatureMismatch));
    }

    #[test]
    fn tamper_on_payouts_amount_fails_verify() {
        let key = SigningKey::generate(&mut OsRng);
        let mut signed = sample().sign(&key);
        signed.payouts[0].amount_nanox = 999_999;
        assert_eq!(signed.verify(), Err(VerifyError::SignatureMismatch));
    }

    #[test]
    fn tamper_on_payouts_address_fails_verify() {
        let key = SigningKey::generate(&mut OsRng);
        let mut signed = sample().sign(&key);
        signed.payouts[0].sui_address = "0xdead".into();
        assert_eq!(signed.verify(), Err(VerifyError::SignatureMismatch));
    }

    #[test]
    fn tamper_on_timestamp_fails_verify() {
        let key = SigningKey::generate(&mut OsRng);
        let mut signed = sample().sign(&key);
        signed.timestamp_ms += 1;
        assert_eq!(signed.verify(), Err(VerifyError::SignatureMismatch));
    }

    /// Descriptive-only fields (helper_peer_ids, proof_ids, client_id,
    /// bid_set_hash) are intentionally NOT covered by the v1 signature.
    /// This test pins that contract so a future change that broadens
    /// the signed payload is visible.
    #[test]
    fn metadata_fields_are_not_signed_v1() {
        let key = SigningKey::generate(&mut OsRng);
        let mut signed = sample().sign(&key);
        signed.helper_peer_ids.push(NodePeerId("12D3KooWInjected".into()));
        signed.proof_ids.push([0xaa; 32]);
        signed.client_id = "swapped".into();
        signed.bid_set_hash = [0xbb; 32];
        assert!(signed.verify().is_ok(), "v1 sig covers only the settlement subset");
    }

    #[test]
    fn parse_sui_address_left_pads_hex() {
        let addr = parse_sui_address("0x0abc");
        let mut expected = [0u8; 32];
        expected[30] = 0x0a;
        expected[31] = 0xbc;
        assert_eq!(addr, expected);
    }

    #[test]
    fn parse_sui_address_rejects_odd_length() {
        // hex::decode requires even number of nibbles; odd-length
        // inputs round-trip to all-zeros which will fail signature
        // verification cleanly instead of panicking.
        assert_eq!(parse_sui_address("0xabc"), [0u8; 32]);
    }
}
