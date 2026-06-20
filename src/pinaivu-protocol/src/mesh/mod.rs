//! libp2p protocol surface shared by coordinator and nodes:
//! the composed `PinaivuBehaviour`, gossipsub topic constants, and the
//! request-response CompletionAck protocol. Coordinator-specific wiring
//! (event loop, peer registry, mesh trait) stays in the coordinator crate.

pub mod behaviour;
pub mod completion_proto;
pub mod inference_proto;
pub mod recruit_proto;
pub mod topics;

pub use behaviour::{libp2p_identity_from_ed25519_secret, PinaivuBehaviour, PinaivuBehaviourEvent};
pub use completion_proto::{CompletionAck, CompletionResponse, COMPLETION_PROTOCOL};
pub use inference_proto::{InferenceDispatch, InferenceReply, INFERENCE_PROTOCOL};
pub use recruit_proto::{RecruitRequest, RecruitResponse, RECRUIT_PROTOCOL};
