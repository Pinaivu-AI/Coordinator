//! libp2p request-response protocol carrying:
//!   primary node → helper node : [`RecruitRequest`]
//!   helper node → primary node : [`RecruitResponse`] (with signed
//!                                  [`ProofOfInference`] on accept)
//!
//! Protocol id: `/pinaivu/recruit/1.0.0`. CBOR-encoded over the wire.
//!
//! Used when the primary node decides a job is too large to serve
//! alone and recruits a helper peer-to-peer (the coordinator is not
//! involved; it audits the result via the proofs in the final
//! `CompletionAck`).

use ed25519_dalek::{Signature, Signer, SigningKey, Verifier, VerifyingKey};
use libp2p::StreamProtocol;
use serde::{Deserialize, Serialize};

use crate::{ProofOfInference, RequestId, VerifyError};

pub const RECRUIT_PROTOCOL: StreamProtocol = StreamProtocol::new("/pinaivu/recruit/1.0.0");

/// Sent by the primary node to a candidate helper. Carries the slice
/// of work the helper should run, plus a signature so the helper can
/// confirm the request came from the legitimate primary.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RecruitRequest {
    pub request_id: RequestId,
    pub primary_pubkey: [u8; 32],
    pub model: String,
    pub prompt_chunk: String,
    pub max_price_nanox: u64,
    pub deadline_ms: u64,
    pub signature: Vec<u8>,
}

impl RecruitRequest {
    /// Canonical bytes the primary signs. Excludes `signature` itself.
    pub fn canonical_bytes(&self) -> Vec<u8> {
        #[derive(Serialize)]
        struct Canonical<'a> {
            request_id: &'a RequestId,
            primary_pubkey: &'a [u8; 32],
            model: &'a String,
            prompt_chunk: &'a String,
            max_price_nanox: u64,
            deadline_ms: u64,
        }
        let canonical = Canonical {
            request_id: &self.request_id,
            primary_pubkey: &self.primary_pubkey,
            model: &self.model,
            prompt_chunk: &self.prompt_chunk,
            max_price_nanox: self.max_price_nanox,
            deadline_ms: self.deadline_ms,
        };
        serde_json::to_vec(&canonical)
            .expect("canonical serialisation is infallible for these field types")
    }

    /// Sign with the primary's signing key. Fills `primary_pubkey` and
    /// `signature` in place.
    pub fn sign(mut self, key: &SigningKey) -> Self {
        self.primary_pubkey = key.verifying_key().to_bytes();
        let msg = self.canonical_bytes();
        let sig: Signature = key.sign(&msg);
        self.signature = sig.to_bytes().to_vec();
        self
    }

    /// Verify the primary's signature.
    pub fn verify(&self) -> Result<(), VerifyError> {
        let vk = VerifyingKey::from_bytes(&self.primary_pubkey)
            .map_err(|_| VerifyError::InvalidPublicKey)?;
        let sig = Signature::from_slice(&self.signature)
            .map_err(|_| VerifyError::InvalidSignatureBytes)?;
        vk.verify(&self.canonical_bytes(), &sig)
            .map_err(|_| VerifyError::SignatureMismatch)
    }
}

/// Helper's reply. On accept, carries a `ProofOfInference` signed with
/// the helper's own key — the primary bundles it into the final
/// `CompletionAck` it sends to the coordinator.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RecruitResponse {
    pub accepted: bool,
    pub proof: Option<ProofOfInference>,
    pub reason: Option<String>,
}

impl RecruitResponse {
    pub fn accept(proof: ProofOfInference) -> Self {
        Self {
            accepted: true,
            proof: Some(proof),
            reason: None,
        }
    }

    pub fn reject(reason: impl Into<String>) -> Self {
        Self {
            accepted: false,
            proof: None,
            reason: Some(reason.into()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ed25519_dalek::SigningKey;
    use rand::rngs::OsRng;
    use uuid::Uuid;

    fn sample() -> RecruitRequest {
        RecruitRequest {
            request_id: Uuid::nil(),
            primary_pubkey: [0u8; 32],
            model: "qwen-72b".into(),
            prompt_chunk: "second half of the prompt".into(),
            max_price_nanox: 500,
            deadline_ms: 1_700_000_000_000,
            signature: Vec::new(),
        }
    }

    #[test]
    fn request_sign_verify_roundtrip() {
        let key = SigningKey::generate(&mut OsRng);
        let signed = sample().sign(&key);
        assert_eq!(signed.primary_pubkey, key.verifying_key().to_bytes());
        assert_eq!(signed.signature.len(), 64);
        assert!(signed.verify().is_ok());
    }

    #[test]
    fn tampering_with_prompt_chunk_invalidates() {
        let key = SigningKey::generate(&mut OsRng);
        let mut signed = sample().sign(&key);
        signed.prompt_chunk = "something else entirely".into();
        assert_eq!(signed.verify(), Err(VerifyError::SignatureMismatch));
    }

    #[test]
    fn tampering_with_deadline_invalidates() {
        let key = SigningKey::generate(&mut OsRng);
        let mut signed = sample().sign(&key);
        signed.deadline_ms = signed.deadline_ms.wrapping_add(1);
        assert_eq!(signed.verify(), Err(VerifyError::SignatureMismatch));
    }
}
