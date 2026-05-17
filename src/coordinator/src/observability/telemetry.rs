//! Tracing subscriber setup.
//!
//! Writes to **stderr** (unbuffered) rather than stdout so log lines
//! survive even when the process dies abruptly — Rust's stdout is
//! block-buffered when redirected to a file (which the enclave init
//! does), so anything in the 4 KiB userspace buffer at exit is lost.

use tracing_subscriber::{fmt, prelude::*, EnvFilter};

pub fn init() {
    let filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| {
        EnvFilter::new("coordinator=info,tower_http=info,libp2p_gossipsub=info,libp2p_swarm=info")
    });
    tracing_subscriber::registry()
        .with(fmt::layer().with_writer(std::io::stderr))
        .with(filter)
        .init();

    // Make any panic surface in the same log stream — without this, a
    // libp2p / tokio task panic would die silently and leave us blind.
    std::panic::set_hook(Box::new(|info| {
        eprintln!("PANIC: {info}");
        eprintln!(
            "backtrace:\n{}",
            std::backtrace::Backtrace::force_capture()
        );
    }));
}
