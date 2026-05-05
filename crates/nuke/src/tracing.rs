//! Tracing initialization helper.
//!
//! Calling [`init`] sets up a `tracing-subscriber` that honors the
//! `RUST_LOG` env var (defaulting to `info`). Idempotent: a second call
//! after a successful first init is a no-op.

use tracing_subscriber::{EnvFilter, fmt};

/// Initialize the global tracing subscriber.
///
/// Honors `RUST_LOG`; defaults to `info`. Returns `true` if this call
/// performed the initialization, `false` if a subscriber was already set.
pub fn init() -> bool {
    let filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info"));
    fmt().with_env_filter(filter).try_init().is_ok()
}
