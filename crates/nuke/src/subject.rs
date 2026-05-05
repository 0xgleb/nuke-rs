//! The [`Subject`] trait.

use std::fmt::{Debug, Display};

/// Typed marker for one dep a reactor reacts to.
///
/// Carries the associated types and consts the framework needs to
/// compose a typed dep list and route decoded events back to the
/// reactor; carries no I/O methods of its own. Transport-specific
/// extension traits add the methods needed to actually open a
/// subscription / decode a wire frame.
pub trait Subject: Send + Sync + 'static {
    /// Strongly-typed identifier for one instance of this dep family.
    /// Distinct `Subject` types use distinct `Id` types, so ids cannot
    /// be mixed up at compile time.
    type Id: Debug + Display + Clone + Send + Sync + 'static;

    /// Decoded event type produced by this dep.
    type Event: Debug + Send + Sync + 'static;

    /// Stable identifier for routing / telemetry; must not change
    /// after downstream consumers persist references to it.
    const NAME: &'static str;

    /// Bumped when this dep's event schema changes. Reserved for the
    /// schema reconciler; the const is here so the API is stable from
    /// day one.
    const SCHEMA_VERSION: u64;
}
