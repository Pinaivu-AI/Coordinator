//! `AppState` — Arc-shared handles wired through axum's `State` extractor.
//!
//! Scaffold — fields land as services come online.

use std::sync::Arc;

#[derive(Clone, Default)]
pub struct AppState {
    inner: Arc<Inner>,
}

#[derive(Default)]
struct Inner {
    // TODO: config, identity, mesh handle, marketplace, apalis storage,
    // postgres pool, redis pool, reputation cache, settlement adapters
}

impl AppState {
    pub fn new() -> Self {
        Self {
            inner: Arc::new(Inner::default()),
        }
    }
}
