//! nuke - general-purpose, event-driven framework. Source -> cqrs/es
//! -> Reactor -> apalis Job DAG -> external services. The framework
//! crate defines the abstract traits + the eDSL + the apalis run-loop,
//! and is venue-agnostic - EVM/SVM/CEX adapters live in sibling
//! crates (e.g. `evm`); persistence lives in the `event-sorcery`
//! crate.
//!
//! See `CLAUDE.md` and `docs/architecture.md` at the repo root for
//! the durable architectural reference. Public vocabulary at a
//! glance:
//!
//! - [`Subject`] - typed marker for an event source the reactor
//!   cares about. Adapter crates extend it (e.g. `evm::EvmSubject`)
//!   with venue-specific subscription / decode methods.
//! - [`Reactor`] - what reacts to events from a *list* of subjects;
//!   the event type is *computed* from the list (no manual enum).
//! - [`subjects!`] - declares a reactor's subject list once and
//!   generates the [`Subscribed`] / [`HasSubject`] impls.
//! - [`pump_through_apalis`](apalis::pump_through_apalis) - the
//!   venue-agnostic run loop adapter crates feed.

// Make `::nuke::*` paths in proc-macro-emitted code resolve when used
// from within this crate (tests that don't go through cargo
// resolution by name). External users get this for free via cargo.
extern crate self as nuke;

pub mod apalis;
pub mod domain;
pub mod error;
pub mod job;
pub mod policy;
pub mod tracing;

mod feed;
mod has_subject;
mod ledger;
mod macros;
mod one_of;
mod reactor;
mod subject;
mod subscribed;
mod venue;

pub use apalis::{PipelineError, pump_through_apalis};
pub use error::{Error, Result};
pub use feed::{Feed, FeedStream};
pub use has_subject::HasSubject;
pub use job::{Job, Label, work};
pub use ledger::Ledger;
pub use nuke_derive::Domain;
pub use one_of::{Fold, OneOf};
pub use reactor::Reactor;
pub use subject::Subject;
pub use subscribed::{Cons, Never, Nil, SubjectList, Subscribed};
pub use venue::{TradingVenue, Venue};

/// Re-exports used by macro expansions. Not part of the supported API.
#[doc(hidden)]
pub mod reexports {
    pub use async_trait::async_trait;
    pub use linkme;
}

/// Common imports for users of the framework.
///
/// Intentionally does **not** export `Error`/`Result` — using the
/// prelude would shadow `std::result::Result`, breaking call sites
/// that mix in other error types (e.g. macro-generated code from
/// `secretspec`). Reach for `nuke::Error` / `nuke::Result` explicitly
/// when needed.
pub mod prelude {
    pub use crate::has_subject::HasSubject;
    pub use crate::one_of::{Fold, OneOf};
    pub use crate::reactor::Reactor;
    pub use crate::subject::Subject;
    pub use crate::subjects;
    pub use crate::subscribed::{Cons, Never, Nil, SubjectList, Subscribed};
    pub use async_trait::async_trait;
}
