//! HTTP surface served on VSOCK port 4000. Speaks an OpenAI-shaped
//! entry path plus Pinaivu-native verification endpoints.

pub mod admin;
pub mod health;
pub mod inference;
pub mod nodes;
pub mod proofs;
