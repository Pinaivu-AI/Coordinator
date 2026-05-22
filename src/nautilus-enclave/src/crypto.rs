//! Enclave Ed25519 + X25519 keypair.
//!
//! Generated fresh on every enclave boot from `OsRng` (which, inside a
//! Nitro Enclave, is fed by the NSM hardware entropy source). The
//! private key never leaves the enclave; the public key is bound into
//! the attestation document so clients can verify they're talking to
//! the expected build.
//!
//! The same seed is also converted to an X25519 static secret for
//! ECDH-based prompt encryption. Clients can fetch the X25519 public
//! key from `GET /enclave_health`, generate an ephemeral keypair,
//! and encrypt their messages array before posting to
//! `POST /v1/chat/completions`. The coordinator decrypts inside the
//! Nitro Enclave so the operator never sees plaintext prompts.
//!
//! Conversion: SHA-512(ed25519_seed)[0..32], matching the standard
//! used by Signal / libsodium / OpenSSH.

use ed25519_dalek::{Signature, Signer, SigningKey, VerifyingKey};
use rand::rngs::OsRng;
use sha2::{Digest, Sha256, Sha512};
use x25519_dalek::{PublicKey as X25519PublicKey, StaticSecret};

pub struct EnclaveKeyPair {
    signing_key:   SigningKey,
    x25519_secret: StaticSecret,
}

impl EnclaveKeyPair {
    /// Generate a fresh keypair from the OS RNG.
    pub fn generate() -> Self {
        let signing_key = SigningKey::generate(&mut OsRng);
        let x25519_secret = ed25519_seed_to_x25519(&signing_key.to_bytes());
        Self { signing_key, x25519_secret }
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

    /// Raw 32-byte Ed25519 secret. Same trust caveat as
    /// [`signing_key`]: callers must already be inside the enclave
    /// trust boundary. Used to seed the libp2p identity so the
    /// coordinator's network PeerId is derived from the same key
    /// that's bound into its attestation document.
    pub fn secret_bytes(&self) -> [u8; 32] {
        self.signing_key.to_bytes()
    }

    // ── X25519 / ECDH ────────────────────────────────────────────────────────

    /// X25519 public key derived from the same seed.
    /// Advertised to clients via `GET /enclave_health` so they can
    /// encrypt messages before sending to `POST /v1/chat/completions`.
    pub fn x25519_public_key(&self) -> X25519PublicKey {
        X25519PublicKey::from(&self.x25519_secret)
    }

    /// ECDH with a client's ephemeral X25519 public key.
    /// Returns a 32-byte shared secret for key derivation.
    /// Only called inside the enclave trust boundary.
    pub fn ecdh(&self, client_pub: &X25519PublicKey) -> [u8; 32] {
        self.x25519_secret.diffie_hellman(client_pub).to_bytes()
    }

    /// Derive a 32-byte AES-256-GCM key from an ECDH shared secret.
    ///
    /// `SHA-256("pinaivu-aes-key-v1" ‖ shared_secret)`
    ///
    /// Must match the client SDK's `deriveAesKey` implementation exactly.
    pub fn derive_aes_key(shared_secret: &[u8; 32]) -> [u8; 32] {
        let mut h = Sha256::new();
        h.update(b"pinaivu-aes-key-v1");
        h.update(shared_secret);
        h.finalize().into()
    }
}

/// Convert an Ed25519 seed to an X25519 static secret via SHA-512.
/// Matches the convention used by Signal, libsodium, and OpenSSH.
fn ed25519_seed_to_x25519(seed: &[u8; 32]) -> StaticSecret {
    let hash = Sha512::digest(seed);
    let mut scalar = [0u8; 32];
    scalar.copy_from_slice(&hash[..32]);
    // x25519-dalek clamps internally during DH.
    StaticSecret::from(scalar)
}

#[cfg(test)]
mod tests {
    use super::*;
    use ed25519_dalek::Verifier;
    use x25519_dalek::EphemeralSecret;

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

    #[test]
    fn x25519_keys_differ_across_keypairs() {
        let a = EnclaveKeyPair::generate();
        let b = EnclaveKeyPair::generate();
        assert_ne!(
            a.x25519_public_key().to_bytes(),
            b.x25519_public_key().to_bytes()
        );
    }

    #[test]
    fn ecdh_shared_secret_matches() {
        let enclave = EnclaveKeyPair::generate();
        let enclave_x25519_pub = enclave.x25519_public_key();

        // Client side: ephemeral keypair
        let client_priv = EphemeralSecret::random_from_rng(OsRng);
        let client_pub  = X25519PublicKey::from(&client_priv);

        let client_shared = client_priv.diffie_hellman(&enclave_x25519_pub).to_bytes();
        let enclave_shared = enclave.ecdh(&client_pub);

        assert_eq!(client_shared, enclave_shared);
    }

    #[test]
    fn derive_aes_key_is_deterministic() {
        let secret = [0x42u8; 32];
        let key1 = EnclaveKeyPair::derive_aes_key(&secret);
        let key2 = EnclaveKeyPair::derive_aes_key(&secret);
        assert_eq!(key1, key2);
    }

    #[test]
    fn derive_aes_key_differs_for_different_secrets() {
        let key1 = EnclaveKeyPair::derive_aes_key(&[0u8; 32]);
        let key2 = EnclaveKeyPair::derive_aes_key(&[1u8; 32]);
        assert_ne!(key1, key2);
    }
}
