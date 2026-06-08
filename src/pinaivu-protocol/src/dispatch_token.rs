//! Signed dispatch token.
//!
//! Issued by the coordinator to a client after auction. The client
//! forwards it to the chosen primary node, which verifies the
//! coordinator's signature before serving the request.

use ed25519_dalek::{Signature, Signer, SigningKey, Verifier, VerifyingKey};
use serde::{Deserialize, Serialize};

use super::types::{NanoX, NodePeerId, RequestId, SessionId};
use super::VerifyError;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DispatchToken {
    pub request_id: RequestId,
    pub session_id: SessionId,
    pub client_pubkey: [u8; 32],
    pub primary_peer_id: NodePeerId,
    pub settlement_id: String,
    pub max_price_nanox: NanoX,
    pub issued_at_ms: u64,
    pub deadline_ms: u64,
    pub coordinator_pubkey: [u8; 32],
    /// X25519 public key of the winning node, forwarded from its bid.
    /// When `Some`, clients must ECDH-encrypt their prompt before
    /// posting to `node_url/v1/inference` — use the same
    /// `SHA-256("pinaivu-aes-key-v1" ‖ shared)` → AES-256-GCM scheme
    /// described in the coordinator's `/v1/chat/completions` docs.
    /// `None` means the node only accepts plaintext prompts.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub node_x25519_pubkey: Option<[u8; 32]>,
    pub signature: Vec<u8>,
}

impl DispatchToken {
    pub fn canonical_bytes(&self) -> Vec<u8> {
        #[derive(Serialize)]
        struct Canonical<'a> {
            request_id: &'a RequestId,
            session_id: &'a SessionId,
            client_pubkey: &'a [u8; 32],
            primary_peer_id: &'a NodePeerId,
            settlement_id: &'a String,
            max_price_nanox: &'a NanoX,
            issued_at_ms: u64,
            deadline_ms: u64,
            coordinator_pubkey: &'a [u8; 32],
            node_x25519_pubkey: &'a Option<[u8; 32]>,
        }
        let canonical = Canonical {
            request_id: &self.request_id,
            session_id: &self.session_id,
            client_pubkey: &self.client_pubkey,
            primary_peer_id: &self.primary_peer_id,
            settlement_id: &self.settlement_id,
            max_price_nanox: &self.max_price_nanox,
            issued_at_ms: self.issued_at_ms,
            deadline_ms: self.deadline_ms,
            coordinator_pubkey: &self.coordinator_pubkey,
            node_x25519_pubkey: &self.node_x25519_pubkey,
        };
        serde_json::to_vec(&canonical)
            .expect("canonical serialisation is infallible for these field types")
    }

    /// Fill `coordinator_pubkey` and `signature` from `key` and return
    /// the signed token. Any existing values are overwritten.
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

    fn sample() -> DispatchToken {
        DispatchToken {
            request_id: uuid::Uuid::nil(),
            session_id: uuid::Uuid::nil(),
            client_pubkey: [3u8; 32],
            primary_peer_id: NodePeerId("12D3KooWPrimary".into()),
            settlement_id: "free".into(),
            max_price_nanox: NanoX(10_000),
            issued_at_ms: 1_700_000_000_000,
            deadline_ms: 1_700_000_005_000,
            coordinator_pubkey: [0u8; 32],
            node_x25519_pubkey: None,
            signature: Vec::new(),
        }
    }

    #[test]
    fn sign_verify_roundtrip() {
        let key = SigningKey::generate(&mut OsRng);
        let signed = sample().sign(&key);
        assert_eq!(signed.coordinator_pubkey, key.verifying_key().to_bytes());
        assert!(signed.verify().is_ok());
    }

    #[test]
    fn tamper_on_primary_peer_fails_verify() {
        let key = SigningKey::generate(&mut OsRng);
        let mut signed = sample().sign(&key);
        signed.primary_peer_id = NodePeerId("12D3KooWAttacker".into());
        assert_eq!(signed.verify(), Err(VerifyError::SignatureMismatch));
    }

    #[test]
    fn tamper_on_deadline_fails_verify() {
        let key = SigningKey::generate(&mut OsRng);
        let mut signed = sample().sign(&key);
        signed.deadline_ms = u64::MAX;
        assert_eq!(signed.verify(), Err(VerifyError::SignatureMismatch));
    }

    #[test]
    fn tamper_on_max_price_fails_verify() {
        let key = SigningKey::generate(&mut OsRng);
        let mut signed = sample().sign(&key);
        signed.max_price_nanox = NanoX(u64::MAX);
        assert_eq!(signed.verify(), Err(VerifyError::SignatureMismatch));
    }
}
