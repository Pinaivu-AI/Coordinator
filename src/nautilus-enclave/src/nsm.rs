//! Nitro Security Module attestation.
//!
//! With the `aws` feature, calls the NSM driver to produce a real
//! COSE_Sign1 attestation document with measured PCRs. Without the
//! feature, returns a deterministic mock so local development works
//! without enclave hardware.

use anyhow::Result;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

/// A coordinator attestation document.
///
/// PCR fields are hex-encoded SHA-384 digests (48 bytes / 96 hex chars)
/// in real NSM docs. The mock path substitutes SHA-256 padded to 48
/// bytes so the shape is the same.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AttestationDoc {
    pub pcr0: String,
    pub pcr1: String,
    pub pcr2: String,
    pub public_key: String,
    pub timestamp_ms: u64,
    /// Hex-encoded COSE_Sign1 document for real attestations; empty
    /// in mock mode (no real signing chain to embed).
    pub raw_cbor_hex: String,
}

/// Produce an attestation binding `public_key` (and `nonce`) to the
/// enclave's PCRs. Returns `Err` if the NSM call fails — callers
/// surface it as HTTP 500 rather than panicking.
pub fn get_attestation(public_key: &[u8; 32], nonce: &[u8]) -> Result<AttestationDoc> {
    let timestamp_ms = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0);

    #[cfg(feature = "aws")]
    {
        use aws_nitro_enclaves_nsm_api::api::{Request, Response};
        use aws_nitro_enclaves_nsm_api::driver;

        let nsm_fd = driver::nsm_init();
        anyhow::ensure!(
            nsm_fd >= 0,
            "NSM device open failed (fd={nsm_fd}). Not running inside a Nitro Enclave?"
        );

        let request = Request::Attestation {
            user_data: if nonce.is_empty() {
                None
            } else {
                Some(nonce.to_vec().into())
            },
            nonce: None,
            public_key: Some(public_key.to_vec().into()),
        };

        let read_pcr = |index: u16| -> Result<String> {
            match driver::nsm_process_request(nsm_fd, Request::DescribePCR { index }) {
                Response::DescribePCR { lock: _, data } => Ok(hex::encode(&data)),
                Response::Error(e) => Err(anyhow::anyhow!("read PCR{index} rejected: {e:?}")),
                other => Err(anyhow::anyhow!("unexpected PCR{index} response: {other:?}")),
            }
        };

        let result = match driver::nsm_process_request(nsm_fd, request) {
            Response::Attestation { document } => {
                let raw_cbor_hex = hex::encode(&document);
                let pcr0 = read_pcr(0)?;
                let pcr1 = read_pcr(1)?;
                let pcr2 = read_pcr(2)?;
                Ok(AttestationDoc {
                    pcr0,
                    pcr1,
                    pcr2,
                    public_key: hex::encode(public_key),
                    timestamp_ms,
                    raw_cbor_hex,
                })
            }
            Response::Error(e) => Err(anyhow::anyhow!("NSM attestation rejected: {e:?}")),
            other => Err(anyhow::anyhow!("unexpected NSM response: {other:?}")),
        };

        driver::nsm_exit(nsm_fd);
        result
    }

    #[cfg(not(feature = "aws"))]
    {
        let mk_pcr = |tag: &[u8]| -> String {
            let mut h = Sha256::new();
            h.update(tag);
            h.update(public_key);
            h.update(nonce);
            let digest = h.finalize();
            let mut out = [0u8; 48];
            out[..32].copy_from_slice(&digest);
            hex::encode(out)
        };

        Ok(AttestationDoc {
            pcr0: mk_pcr(b"pcr0"),
            pcr1: mk_pcr(b"pcr1"),
            pcr2: mk_pcr(b"pcr2"),
            public_key: hex::encode(public_key),
            timestamp_ms,
            raw_cbor_hex: String::new(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pcrs_are_48_bytes_hex() {
        let doc = get_attestation(&[0u8; 32], b"nonce").unwrap();
        assert_eq!(doc.pcr0.len(), 96);
        assert_eq!(doc.pcr1.len(), 96);
        assert_eq!(doc.pcr2.len(), 96);
    }

    #[test]
    fn different_pubkey_changes_pcrs() {
        let a = get_attestation(&[0u8; 32], b"").unwrap();
        let b = get_attestation(&[1u8; 32], b"").unwrap();
        assert_ne!(a.pcr0, b.pcr0);
    }
}
