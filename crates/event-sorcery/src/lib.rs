//! Ergonomic typed wrapper around [`cqrs-es`][cqrs-es].
//!
//! cqrs-es's `Aggregate` trait has sharp edges that have caused
//! production bugs (infallible `apply`, stringly aggregate IDs, no
//! schema versioning, flat command handling). This crate exposes a
//! small set of typed traits that capture the lessons:
//!
//! - [`EventSourced`] - user-facing trait with rich associated types
//!   and consts. Naming asymmetry (`originate`/`evolve` for
//!   event-side; `initialize`/`transition` for command-side) marks
//!   the domain boundary.
//! - [`Lifecycle<E>`] - internal adapter providing the blanket
//!   `cqrs_es::Aggregate` impl. Users never touch cqrs-es directly.
//! - [`Never`] - uninhabited error for entities whose operations
//!   can't fail.
//!
//! [cqrs-es]: https://crates.io/crates/cqrs-es

// `extern crate self as event_sorcery` makes `$crate::*` paths in our
// own macros resolve when the macro is invoked from inside this crate
// (the test module, doctests). External users get this for free via
// cargo's crate name resolution.
extern crate self as event_sorcery;

pub mod dep;
pub mod event_sourced;
pub mod lifecycle;

pub use dep::{Cons, Dep, DepList, Dependent, Fold, HasDep, Nil, OneOf};
pub use event_sourced::{DomainError, EventSourced};
pub use lifecycle::{Lifecycle, LifecycleError};

// Lifecycle's `Never` and dep's `Never` are the same concept (an
// uninhabited type). Re-export the dep module's version under the
// crate root and keep the lifecycle one available via its module
// path for back-compat. Future cleanup: collapse to one.
pub use dep::Never;
