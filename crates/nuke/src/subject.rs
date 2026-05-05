//! The [`Subject`] trait — typed marker for an event source the
//! reactor cares about.
//!
//! Subject is intentionally **venue-agnostic**: it carries only the
//! types and constants the framework needs to compose the type-level
//! subscription list (`Subscribed::Subjects`) and the discriminated
//! event union (`SubjectList::Event`). Venue-specific concerns —
//! addresses, subscription params, decoding — live in adapter
//! crates' own traits that *extend* `Subject` (e.g.
//! `evm::EvmSubject: nuke::Subject`).
//!
//! Most users derive this via an adapter-provided derive (e.g.
//! `#[derive(EvmSubject)]` from the `evm` crate generates both the
//! `nuke::Subject` impl and the `evm::EvmSubject` impl).

use std::fmt::{Debug, Display};

/// Typed marker for an event source the framework can subscribe to.
///
/// `Subject` is not a connection or a stream — it's just the
/// type-level identifier that lets the framework build a typed
/// subscription list (`Subscribed::Subjects`), compute the event
/// union, and route events back into the right reactor handler.
/// Venue-specific subscription / decode methods live on adapter
/// traits that extend `Subject`.
pub trait Subject: Send + Sync + 'static {
    /// Strongly-typed identifier. Prevents id mix-ups across subject
    /// types at compile time. For on-chain subjects this is usually a
    /// newtype around the contract address; for CEX subjects it might
    /// be `(Exchange, Symbol)`; for IoT it might be a sensor id.
    type Id: Debug + Display + Clone + Send + Sync + 'static;

    /// Decoded event type. Adapter-specific (e.g. for `evm` it's the
    /// `alloy::sol!`-generated event struct).
    type Event: Debug + Send + Sync + 'static;

    /// Stable identifier for routing / telemetry; must not change
    /// after downstream consumers persist references to it.
    const NAME: &'static str;

    /// Bumped when the subject's event schema changes. Reserved for
    /// the schema reconciler (see ROADMAP framework-deepening epic);
    /// the const is in v0 so the API is stable from day one.
    const SCHEMA_VERSION: u64;
}
