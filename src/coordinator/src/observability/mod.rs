//! Observability surface — tracing, metrics, structured logging.
//! Currently houses the tracing initializer; metrics endpoints land
//! alongside the api/health module.

pub mod telemetry;

pub use telemetry::init;
