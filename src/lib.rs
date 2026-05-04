//! nuke-rs — a Tower-style, extensible framework for trading/crypto
//! websocket communication with apalis-backed event execution.
//!
//! See the README and `examples/arb_bot/main.rs` for the user-facing shape.
//! The framework's public vocabulary is two traits and one declarative
//! macro:
//!
//! - [`Subject`] — something on a chain that emits typed events.
//! - [`Reactor`] — what reacts to events from a *list* of subjects, with
//!   the event type computed from the list (no manual enum).
//! - [`subjects!`] — declares a reactor's subject list once and generates
//!   the [`Subscribed`] and [`HasSubject`] impls.

pub mod error;
pub mod evm;
pub mod tracing;

mod has_subject;
mod macros;
mod one_of;
mod reactor;
mod subject;
mod subscribed;

pub use error::{Error, Result};
pub use has_subject::HasSubject;
pub use nuke_derive::EvmSubject;
pub use one_of::{Fold, OneOf};
pub use reactor::Reactor;
pub use subject::Subject;
pub use subscribed::{Cons, Never, Nil, SubjectList, Subscribed};

/// Re-exports used by macro expansions. Not part of the supported API.
#[doc(hidden)]
pub mod reexports {
    pub use alloy_primitives;
    pub use alloy_sol_types;
    pub use async_trait::async_trait;
}

/// Common imports for users of the framework.
///
/// Intentionally does **not** export `Error`/`Result` — using the prelude
/// would shadow `std::result::Result`, breaking call sites that mix in
/// other error types (e.g. macro-generated code from `secretspec`).
/// Reach for `nuke::Error` / `nuke::Result` explicitly when needed.
pub mod prelude {
    pub use crate::has_subject::HasSubject;
    pub use crate::one_of::{Fold, OneOf};
    pub use crate::reactor::Reactor;
    pub use crate::subject::Subject;
    pub use crate::subjects;
    pub use crate::subscribed::{Cons, Never, Nil, SubjectList, Subscribed};
    pub use async_trait::async_trait;
    pub use nuke_derive::EvmSubject;
}

/// Run a reactor against an EVM JSON-RPC websocket source.
///
/// Reads `R::Subjects` at compile time, opens the corresponding
/// subscriptions on `source`, and pumps decoded events into
/// `reactor.react(...)` until the source ends or an error occurs.
pub async fn run<R>(source: evm::EvmWsSource, reactor: std::sync::Arc<R>) -> Result<()>
where
    R: Reactor + 'static,
    R::Subjects: evm::Subscribe<R::Subjects>,
{
    evm::pump(source, reactor).await
}
