//! Enclave Ed25519 keypair.
//!
//! Generated fresh on every enclave boot from `OsRng` (which, inside a
//! Nitro Enclave, is fed by the NSM hardware entropy source). The
//! private key never leaves the enclave; the public key is bound into
//! the attestation document so clients can verify they're talking to
//! the expected build.

use ed25519_dalek::{Signature, Signer, SigningKey, VerifyingKey};
use rand::rngs::OsRng;

pub struct EnclaveKeyPair {
    signing_key: SigningKey,
}

impl EnclaveKeyPair {
    /// Generate a fresh keypair from the OS RNG.
    pub fn generate() -> Self {
        let signing_key = SigningKey::generate(&mut OsRng);
        Self { signing_key }
    }

    pub fn public_key_bytes(&self) -> [u8; 32] {
        self.signing_key.verifying_key().to_bytes()
    }

    pub fn verifying_key(&self) -> VerifyingKey {
        self.signing_key.verifying_key()
    }

    pub fn sign(&self, msg: &[u8]) -> Signature {
        self.signing_key.sign(msg)
    }

    /// Access the underlying `SigningKey`. Callers must already be
    /// inside the enclave trust boundary — exposing this lets us
    /// share signing across protocol artefacts without each one
    /// having to be wrapped in a bespoke helper.
    pub fn signing_key(&self) -> &SigningKey {
        &self.signing_key
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ed25519_dalek::Verifier;

    #[test]
    fn generates_unique_keys() {
        let a = EnclaveKeyPair::generate();
        let b = EnclaveKeyPair::generate();
        assert_ne!(a.public_key_bytes(), b.public_key_bytes());
    }

    #[test]
    fn sign_verify_roundtrip() {
        let kp = EnclaveKeyPair::generate();
        let msg = b"pinaivu coordinator scaffold";
        let sig = kp.sign(msg);
        assert!(kp.verifying_key().verify(msg, &sig).is_ok());
    }
}
