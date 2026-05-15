//! Nautilus enclave primitives.
//!
//! Isolates everything that depends on the AWS Nitro Security Module
//! (NSM) and the coordinator's Ed25519 signing identity from the rest
//! of the coordinator crate. Mirrors the structure of
//! `nautilus-memwal-relayer/src/nautilus-enclave/`.
//!
//! - [`crypto`] — Ed25519 keypair (`EnclaveKeyPair`) generated inside
//!   the enclave from NSM-backed entropy. Used for routing receipts,
//!   dispatch tokens, and signed HTTP responses.
//! - [`nsm`] — attestation document production. Real impl behind the
//!   `aws` feature; deterministic mock for local dev.

pub mod crypto;
pub mod nsm;

pub use crypto::EnclaveKeyPair;
pub use nsm::{get_attestation, AttestationDoc};
