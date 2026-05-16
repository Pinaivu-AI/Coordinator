//! libp2p request-response protocol carrying:
//!   node → coordinator : [`CompletionAck`]
//!   coordinator → node : [`CompletionResponse`] (signed routing receipt)
//!
//! Protocol id: `/pinaivu/completion/1.0.0`. CBOR-encoded over the
//! wire — smaller than JSON for the byte-array-heavy proof payloads.

use ed25519_dalek::{Signature, Signer, SigningKey, Verifier, VerifyingKey};
use libp2p::StreamProtocol;
use serde::{Deserialize, Serialize};

use crate::protocol::{ProofOfInference, RequestId, RoutingReceipt, VerifyError};

pub const COMPLETION_PROTOCOL: StreamProtocol =
    StreamProtocol::new("/pinaivu/completion/1.0.0");

/// Sent by the primary node to the coordinator after the job has been
/// served to the client. Carries the bundle of `ProofOfInference`s
/// from every contributing node — primary first, helpers after.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CompletionAck {
    pub request_id: RequestId,
    pub proofs: Vec<ProofOfInference>,
    pub aggregated_output_hash: [u8; 32],
    pub primary_pubkey: [u8; 32],
    pub signature: Vec<u8>,
}

impl CompletionAck {
    /// Canonical bytes the primary signs. The full `proofs` payload
    /// is summarised as `proof_ids` (the SHA-256 leaf hashes) so the
    /// signature commits to the proof set without ballooning to
    /// kilobytes of canonical JSON.
    pub fn canonical_bytes(&self) -> Vec<u8> {
        #[derive(Serialize)]
        struct Canonical<'a> {
            request_id: &'a RequestId,
            proof_ids: Vec<[u8; 32]>,
            aggregated_output_hash: &'a [u8; 32],
            primary_pubkey: &'a [u8; 32],
        }
        let canonical = Canonical {
            request_id: &self.request_id,
            proof_ids: self.proofs.iter().map(|p| p.id()).collect(),
            aggregated_output_hash: &self.aggregated_output_hash,
            primary_pubkey: &self.primary_pubkey,
        };
        serde_json::to_vec(&canonical)
            .expect("canonical serialisation is infallible for these field types")
    }

    /// Sign the ack with the primary node's keypair. Sets
    /// `primary_pubkey` and `signature` in place.
    pub fn sign(mut self, key: &SigningKey) -> Self {
        self.primary_pubkey = key.verifying_key().to_bytes();
        let msg = self.canonical_bytes();
        let sig: Signature = key.sign(&msg);
        self.signature = sig.to_bytes().to_vec();
        self
    }

    /// Verify the primary node's signature over the ack.
    pub fn verify_primary(&self) -> Result<(), VerifyError> {
        let vk = VerifyingKey::from_bytes(&self.primary_pubkey)
            .map_err(|_| VerifyError::InvalidPublicKey)?;
        let sig = Signature::from_slice(&self.signature)
            .map_err(|_| VerifyError::InvalidSignatureBytes)?;
        vk.verify(&self.canonical_bytes(), &sig)
            .map_err(|_| VerifyError::SignatureMismatch)
    }

    /// Verify every embedded proof's signature.
    pub fn verify_all_proofs(&self) -> Result<(), VerifyError> {
        for proof in &self.proofs {
            proof.verify()?;
        }
        Ok(())
    }

    pub fn proof_ids(&self) -> Vec<[u8; 32]> {
        self.proofs.iter().map(|p| p.id()).collect()
    }
}

/// Returned by the coordinator after processing a [`CompletionAck`].
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CompletionResponse {
    pub accepted: bool,
    pub routing_receipt: Option<RoutingReceipt>,
    pub reason: Option<String>,
}

impl CompletionResponse {
    pub fn ok(receipt: RoutingReceipt) -> Self {
        Self {
            accepted: true,
            routing_receipt: Some(receipt),
            reason: None,
        }
    }

    pub fn rejected(reason: impl Into<String>) -> Self {
        Self {
            accepted: false,
            routing_receipt: None,
            reason: Some(reason.into()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::protocol::{NanoX, NodePeerId};
    use ed25519_dalek::SigningKey;
    use rand::rngs::OsRng;
    use uuid::Uuid;

    fn sample_proof(peer: &str, key: &SigningKey) -> ProofOfInference {
        ProofOfInference {
            request_id: Uuid::nil(),
            session_id: Uuid::nil(),
            node_peer_id: NodePeerId(peer.into()),
            client_address: "client-test".into(),
            model_id: "qwen-72b".into(),
            input_tokens: 64,
            output_tokens: 128,
            latency_ms: 300,
            price_paid_nanox: NanoX(1_000),
            timestamp: 1,
            input_hash: [1u8; 32],
            output_hash: [2u8; 32],
            settlement_id: "free".into(),
            escrow_tx_id: None,
            node_pubkey: [0u8; 32],
            signature: Vec::new(),
        }
        .sign(key)
    }

    #[test]
    fn ack_sign_verify_roundtrip() {
        let key = SigningKey::generate(&mut OsRng);
        let proof = sample_proof("PRIMARY", &key);
        let ack = CompletionAck {
            request_id: Uuid::nil(),
            proofs: vec![proof],
            aggregated_output_hash: [9u8; 32],
            primary_pubkey: [0u8; 32],
            signature: Vec::new(),
        }
        .sign(&key);
        assert!(ack.verify_primary().is_ok());
        assert!(ack.verify_all_proofs().is_ok());
    }

    #[test]
    fn tampering_with_proof_set_invalidates_primary_signature() {
        let key = SigningKey::generate(&mut OsRng);
        let proof = sample_proof("PRIMARY", &key);
        let mut ack = CompletionAck {
            request_id: Uuid::nil(),
            proofs: vec![proof.clone()],
            aggregated_output_hash: [9u8; 32],
            primary_pubkey: [0u8; 32],
            signature: Vec::new(),
        }
        .sign(&key);
        // Add a second proof after signing — canonical_bytes() will
        // now reflect two proof_ids while the signature was over one.
        ack.proofs.push(proof);
        assert_eq!(ack.verify_primary(), Err(VerifyError::SignatureMismatch));
    }

    #[test]
    fn tampering_with_output_hash_invalidates() {
        let key = SigningKey::generate(&mut OsRng);
        let mut ack = CompletionAck {
            request_id: Uuid::nil(),
            proofs: vec![sample_proof("PRIMARY", &key)],
            aggregated_output_hash: [9u8; 32],
            primary_pubkey: [0u8; 32],
            signature: Vec::new(),
        }
        .sign(&key);
        ack.aggregated_output_hash = [0xffu8; 32];
        assert_eq!(ack.verify_primary(), Err(VerifyError::SignatureMismatch));
    }
}
