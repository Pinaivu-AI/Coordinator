//! `ProofOfInference` — a node-signed execution receipt.
//!
//! Self-verifiable: a holder of `(proof, node_pubkey)` can verify the
//! signature offline with no network or chain access.

use ed25519_dalek::{Signature, Signer, SigningKey, Verifier, VerifyingKey};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use super::types::{NanoX, NodePeerId, RequestId, SessionId};
use super::VerifyError;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProofOfInference {
    pub request_id: RequestId,
    pub session_id: SessionId,
    pub node_peer_id: NodePeerId,
    pub client_address: String,
    pub model_id: String,
    pub input_tokens: u32,
    pub output_tokens: u32,
    pub latency_ms: u32,
    pub price_paid_nanox: NanoX,
    pub timestamp: u64,
    pub input_hash: [u8; 32],
    pub output_hash: [u8; 32],
    pub settlement_id: String,
    pub escrow_tx_id: Option<String>,
    pub node_pubkey: [u8; 32],
    pub signature: Vec<u8>,
}

impl ProofOfInference {
    /// Canonical bytes that the node signs. Excludes the `signature`
    /// field itself; every other field is included in declaration order
    /// via serde's struct serialiser.
    pub fn canonical_bytes(&self) -> Vec<u8> {
        #[derive(Serialize)]
        struct Canonical<'a> {
            request_id: &'a RequestId,
            session_id: &'a SessionId,
            node_peer_id: &'a NodePeerId,
            client_address: &'a String,
            model_id: &'a String,
            input_tokens: u32,
            output_tokens: u32,
            latency_ms: u32,
            price_paid_nanox: &'a NanoX,
            timestamp: u64,
            input_hash: &'a [u8; 32],
            output_hash: &'a [u8; 32],
            settlement_id: &'a String,
            escrow_tx_id: &'a Option<String>,
            node_pubkey: &'a [u8; 32],
        }
        let canonical = Canonical {
            request_id: &self.request_id,
            session_id: &self.session_id,
            node_peer_id: &self.node_peer_id,
            client_address: &self.client_address,
            model_id: &self.model_id,
            input_tokens: self.input_tokens,
            output_tokens: self.output_tokens,
            latency_ms: self.latency_ms,
            price_paid_nanox: &self.price_paid_nanox,
            timestamp: self.timestamp,
            input_hash: &self.input_hash,
            output_hash: &self.output_hash,
            settlement_id: &self.settlement_id,
            escrow_tx_id: &self.escrow_tx_id,
            node_pubkey: &self.node_pubkey,
        };
        serde_json::to_vec(&canonical)
            .expect("canonical serialisation is infallible for these field types")
    }

    /// SHA-256 over `canonical_bytes()`. Used as the Merkle leaf hash
    /// in a node's reputation tree.
    pub fn id(&self) -> [u8; 32] {
        Sha256::digest(self.canonical_bytes()).into()
    }

    /// Fill `node_pubkey` and `signature` from `key` and return the
    /// signed proof. Any existing values in those fields are
    /// overwritten.
    pub fn sign(mut self, key: &SigningKey) -> Self {
        self.node_pubkey = key.verifying_key().to_bytes();
        // signature field is excluded from canonical_bytes, so we can
        // compute the message before writing the signature back.
        let msg = self.canonical_bytes();
        let sig: Signature = key.sign(&msg);
        self.signature = sig.to_bytes().to_vec();
        self
    }

    /// Verify the embedded Ed25519 signature against `node_pubkey`
    /// and `canonical_bytes()`. Offline, no network or chain access.
    pub fn verify(&self) -> Result<(), VerifyError> {
        let vk = VerifyingKey::from_bytes(&self.node_pubkey)
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

    fn sample() -> ProofOfInference {
        ProofOfInference {
            request_id: uuid::Uuid::nil(),
            session_id: uuid::Uuid::nil(),
            node_peer_id: NodePeerId("12D3KooWPeerNode1".into()),
            client_address: "client-abc".into(),
            model_id: "qwen-72b".into(),
            input_tokens: 128,
            output_tokens: 256,
            latency_ms: 420,
            price_paid_nanox: NanoX(1_000),
            timestamp: 1_700_000_000,
            input_hash: [1u8; 32],
            output_hash: [2u8; 32],
            settlement_id: "free".into(),
            escrow_tx_id: None,
            node_pubkey: [0u8; 32],
            signature: Vec::new(),
        }
    }

    #[test]
    fn sign_verify_roundtrip() {
        let key = SigningKey::generate(&mut OsRng);
        let signed = sample().sign(&key);
        assert_eq!(signed.node_pubkey, key.verifying_key().to_bytes());
        assert_eq!(signed.signature.len(), 64);
        assert!(signed.verify().is_ok());
    }

    #[test]
    fn id_is_deterministic_and_excludes_signature() {
        let key = SigningKey::generate(&mut OsRng);
        let signed = sample().sign(&key);
        let id_a = signed.id();
        let mut tampered_sig = signed.clone();
        tampered_sig.signature = vec![0xff; 64];
        // id() hashes canonical_bytes(), which excludes the signature.
        // So id() must be invariant under signature mutation.
        assert_eq!(id_a, tampered_sig.id());
    }

    #[test]
    fn tamper_on_output_tokens_fails_verify() {
        let key = SigningKey::generate(&mut OsRng);
        let mut signed = sample().sign(&key);
        signed.output_tokens += 1;
        assert_eq!(signed.verify(), Err(VerifyError::SignatureMismatch));
    }

    #[test]
    fn tamper_on_latency_fails_verify() {
        let key = SigningKey::generate(&mut OsRng);
        let mut signed = sample().sign(&key);
        signed.latency_ms = 1;
        assert_eq!(signed.verify(), Err(VerifyError::SignatureMismatch));
    }

    #[test]
    fn tamper_on_output_hash_fails_verify() {
        let key = SigningKey::generate(&mut OsRng);
        let mut signed = sample().sign(&key);
        signed.output_hash = [9u8; 32];
        assert_eq!(signed.verify(), Err(VerifyError::SignatureMismatch));
    }

    #[test]
    fn wrong_pubkey_fails_verify() {
        let key = SigningKey::generate(&mut OsRng);
        let other = SigningKey::generate(&mut OsRng);
        let mut signed = sample().sign(&key);
        signed.node_pubkey = other.verifying_key().to_bytes();
        assert_eq!(signed.verify(), Err(VerifyError::SignatureMismatch));
    }

    #[test]
    fn malformed_signature_bytes_fail_verify() {
        let key = SigningKey::generate(&mut OsRng);
        let mut signed = sample().sign(&key);
        signed.signature = vec![0u8; 10];
        assert_eq!(signed.verify(), Err(VerifyError::InvalidSignatureBytes));
    }

    #[test]
    fn malformed_pubkey_fails_verify() {
        let key = SigningKey::generate(&mut OsRng);
        let mut signed = sample().sign(&key);
        // Pubkey must be a valid Edwards25519 point — set to all-ones
        // which is rejected by from_bytes for many bit patterns.
        signed.node_pubkey = [0xffu8; 32];
        // Either InvalidPublicKey or SignatureMismatch, depending on
        // whether all-ones happens to decode. Either is acceptable —
        // both prove verify() rejected the proof.
        assert!(signed.verify().is_err());
    }
}
