//! Application scaffolding — config loading, error type, shared
//! `AppState` handed to every request handler.

pub mod config;
pub mod error;
pub mod state;

pub use config::Config;
pub use error::AppError;
pub use state::AppState;
