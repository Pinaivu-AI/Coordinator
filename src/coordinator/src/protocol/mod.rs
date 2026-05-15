//! Protocol — wire-format types and coordinator-signed artefacts that
//! travel between client, coordinator, and GPU nodes.

pub mod dispatch_token;
pub mod proof;
pub mod routing_receipt;
pub mod types;

pub use dispatch_token::DispatchToken;
pub use proof::ProofOfInference;
pub use routing_receipt::RoutingReceipt;
pub use types::{
    InferenceBid, InferenceRequest, NanoX, NodeCapabilities, NodePeerId, PrivacyLevel, RequestId,
    SessionId,
};

/// Errors returned by `verify()` on any signed protocol artefact.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum VerifyError {
    #[error("public key bytes are not a valid Ed25519 verifying key")]
    InvalidPublicKey,
    #[error("signature bytes are not a well-formed Ed25519 signature")]
    InvalidSignatureBytes,
    #[error("signature does not verify against the embedded public key")]
    SignatureMismatch,
}
