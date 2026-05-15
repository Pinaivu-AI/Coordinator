//! Nitro Security Module attestation.
//!
//! Real implementation is gated behind the `aws` cargo feature and
//! calls the NSM ioctl interface via the `aws` support crate. Mock
//! implementation (default) returns a deterministic document so
//! local development works without enclave hardware.

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

/// A coordinator attestation document.
///
/// The 48-byte PCR fields are hex-encoded SHA-384 digests in real NSM
/// docs. For the mock path we substitute SHA-256 padded to 48 bytes;
/// the shape is what matters to consumers during scaffolding.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AttestationDoc {
    pub pcr0: String,
    pub pcr1: String,
    pub pcr2: String,
    pub public_key: String,
    pub timestamp_ms: u64,
    pub raw_cbor_hex: String,
}

/// Produce an attestation binding `public_key` (and optional `nonce`)
/// to the enclave's PCRs.
pub fn get_attestation(public_key: &[u8; 32], nonce: &[u8]) -> AttestationDoc {
    #[cfg(feature = "aws")]
    {
        // TODO(aws): call into the `aws` crate's NSM ioctl wrapper to
        // produce a real attestation document.
        let _ = (public_key, nonce);
        unimplemented!("real NSM attestation not yet wired up")
    }

    #[cfg(not(feature = "aws"))]
    {
        mock_attestation(public_key, nonce)
    }
}

#[cfg(not(feature = "aws"))]
fn mock_attestation(public_key: &[u8; 32], nonce: &[u8]) -> AttestationDoc {
    let mk_pcr = |tag: &[u8]| -> String {
        let mut h = Sha256::new();
        h.update(tag);
        h.update(public_key);
        h.update(nonce);
        let digest = h.finalize();
        // Pad SHA-256 (32 bytes) out to 48 bytes to look PCR-shaped.
        let mut out = [0u8; 48];
        out[..32].copy_from_slice(&digest);
        hex::encode(out)
    };

    AttestationDoc {
        pcr0: mk_pcr(b"pcr0"),
        pcr1: mk_pcr(b"pcr1"),
        pcr2: mk_pcr(b"pcr2"),
        public_key: hex::encode(public_key),
        timestamp_ms: 0,
        raw_cbor_hex: String::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pcrs_are_48_bytes_hex() {
        let doc = get_attestation(&[0u8; 32], b"nonce");
        assert_eq!(doc.pcr0.len(), 96);
        assert_eq!(doc.pcr1.len(), 96);
        assert_eq!(doc.pcr2.len(), 96);
    }

    #[test]
    fn different_pubkey_changes_pcrs() {
        let a = get_attestation(&[0u8; 32], b"");
        let b = get_attestation(&[1u8; 32], b"");
        assert_ne!(a.pcr0, b.pcr0);
    }
}
