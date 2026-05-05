//! Event-sourced persistence — a thin wrapper around
//! [`cqrs-es`](https://crates.io/crates/cqrs-es) modeled on
//! `~/code/st0x/st0x.liquidity/crates/event-sorcery/`.
//!
//! cqrs-es's `Aggregate` trait has sharp edges that have caused
//! production bugs (infallible `apply`, stringly aggregate IDs, no
//! schema versioning, flat command handling). This module exposes a
//! small set of nuke-flavoured traits that capture the lessons:
//!
//! - [`EventSourced`] — user-facing trait with rich associated types
//!   and consts. Naming asymmetry (`originate`/`evolve` for
//!   event-side; `initialize`/`transition` for command-side) marks
//!   the domain boundary.
//! - [`Lifecycle<E>`] — internal adapter providing the blanket
//!   `cqrs_es::Aggregate` impl. Users never touch cqrs-es directly.
//! - [`Never`] — uninhabited error for entities whose operations
//!   can't fail.
//!
//! The full cqrs-es bridge (Store, projections, schema reconciler)
//! lands when persistence has actual consumers — see ROADMAP epic
//! "cqrs-es persistence layer".

pub mod event_sourced;
pub mod lifecycle;

pub use event_sourced::{DomainError, EventSourced};
pub use lifecycle::{Lifecycle, LifecycleError, Never};
