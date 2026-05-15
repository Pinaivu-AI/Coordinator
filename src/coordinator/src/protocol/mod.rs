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
