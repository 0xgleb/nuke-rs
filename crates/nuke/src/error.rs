//! Top-level error and result aliases for the framework crate.
//!
//! `Error` is intentionally a small enum of categories the framework
//! itself produces. Adapter-specific decode / protocol errors live in
//! the adapter crates and are wrapped through `Error::Transport` (or
//! `Error::Config`) at the framework boundary.

use std::fmt;

/// Result type used across the public API.
pub type Result<T> = std::result::Result<T, Error>;

/// Errors produced by the framework itself.
///
/// User reactor errors are NOT wrapped here - they are surfaced
/// separately by the run loop so the user can match on their own type.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    /// Underlying transport (network, ws, http) failure surfaced by an
    /// adapter.
    #[error("transport error: {0}")]
    Transport(#[source] Box<dyn std::error::Error + Send + Sync>),

    /// Configuration / startup error (bad URL, missing required value).
    #[error("configuration error: {0}")]
    Config(String),

    /// Reactor signalled an error during job execution.
    #[error("reactor error: {0}")]
    Reactor(#[source] Box<dyn std::error::Error + Send + Sync>),
}

impl Error {
    /// Build a [`Error::Config`] from anything `Display`. Useful for the
    /// example's `unwrap_or_else(|| Error::msg(...))` pattern when a
    /// secret is missing.
    pub fn msg(message: impl fmt::Display) -> Self {
        Self::Config(message.to_string())
    }
}
