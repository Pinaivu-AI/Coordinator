//! On-chain glue. Today this is just the sidecar HTTP client used to
//! submit Sui transactions; settlement-side helpers join here as
//! payment computation lands.

pub mod sui;

pub use sui::{spawn_registration, SidecarClient, RegisteredEnclave};
