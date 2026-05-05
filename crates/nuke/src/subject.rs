//! The [`Subject`] trait.

use std::fmt::{Debug, Display};

use event_sorcery::Dep;

/// Typed marker for one external-stream dep a reactor reacts to.
///
/// `Subject` is a refinement of [`Dep`] (which is the bare-minimum
/// "I have an `Id` and an `Event`" trait). It adds the const metadata
/// the framework needs to route events from a transport back to a
/// typed reactor handler: a stable `NAME` for telemetry and a
/// `SCHEMA_VERSION` for the schema reconciler, plus the `Debug` /
/// `Display` / `Clone` bounds Subject users rely on. Transport-
/// specific extension traits add the methods needed to actually open
/// a subscription / decode a wire frame.
///
/// Internal aggregates (`event_sorcery::EventSourced`) are also
/// [`Dep`]s but not [`Subject`]s; the framework's run loop treats
/// the two families uniformly via [`Dep`] but only [`Subject`]s
/// participate in transport-side wiring.
pub trait Subject: Dep<Id: Debug + Display + Clone, Event: Debug> + Send + Sync + 'static {
    /// Stable identifier for routing / telemetry; must not change
    /// after downstream consumers persist references to it.
    const NAME: &'static str;

    /// Bumped when this dep's event schema changes. Reserved for the
    /// schema reconciler; the const is here so the API is stable
    /// from day one.
    const SCHEMA_VERSION: u64;
}
