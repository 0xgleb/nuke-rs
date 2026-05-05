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

pub mod event_sourced;
pub mod lifecycle;

pub use event_sourced::{DomainError, EventSourced};
pub use lifecycle::{Lifecycle, LifecycleError, Never};
