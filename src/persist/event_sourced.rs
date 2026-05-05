//! The user-facing [`EventSourced`] trait — rich associated types and
//! consts that capture every load-bearing decision about an
//! event-sourced entity. Naming asymmetry between event-side and
//! command-side methods is intentional: events are *facts*, commands
//! are *intent*; different verbs mark different semantics.

use std::fmt::{Debug, Display};
use std::str::FromStr;

use async_trait::async_trait;
use cqrs_es::DomainEvent;
use serde::Serialize;
use serde::de::DeserializeOwned;

/// Bounds required for domain error types stored in the lifecycle.
/// Captures `Clone + Serialize + Deserialize + Send + Sync` in one
/// name so implementors see a single meaningful trait, not a long bound
/// list.
pub trait DomainError:
    std::error::Error + Clone + Serialize + DeserializeOwned + Send + Sync
{
}

impl<T> DomainError for T where
    T: std::error::Error + Clone + Serialize + DeserializeOwned + Send + Sync
{
}

/// The user-facing event-sourced entity trait.
///
/// Implement this on a domain type to get a complete event-sourcing
/// setup; [`Lifecycle`](crate::persist::Lifecycle) provides the
/// blanket `cqrs_es::Aggregate` impl.
#[async_trait]
pub trait EventSourced: Clone + Debug + Send + Sync + Sized + Serialize + DeserializeOwned {
    /// Strongly-typed aggregate identifier. Prevents mixing IDs across
    /// entity types at compile time.
    type Id: Debug + Display + FromStr + Clone + Send + Sync;
    /// Domain events that drive state changes.
    type Event: DomainEvent;
    /// Commands that produce events. One type for both initialization
    /// and transitions — the lifecycle routes by state.
    type Command: Send + Sync;
    /// Domain-specific errors from command handlers / event
    /// application. Use [`crate::persist::Never`] for infallible
    /// entities.
    type Error: DomainError;
    /// External dependencies injected into command handlers (e.g.
    /// `Arc<dyn OrderPlacer>`). Use `()` when none are needed.
    type Services: Send + Sync;

    /// Stable identifier for this aggregate type in the event store.
    /// Must not change after events are persisted.
    const AGGREGATE_TYPE: &'static str;
    /// Bumped when the entity's state, event, or view schema changes.
    /// Reserved for the schema reconciler that lands in a follow-up;
    /// the const is here from day one so the API is stable.
    const SCHEMA_VERSION: u64;

    // --- Event side: replaying events to reconstruct state -----------

    /// Create initial state from a genesis event.
    ///
    /// Returns `Some(state)` for events that *can* originate the
    /// entity, `None` for events that require existing state. Returning
    /// `None` puts the lifecycle into a failed state.
    fn originate(event: &Self::Event) -> Option<Self>;

    /// Derive new state from an event applied to the current entity.
    ///
    /// - `Ok(Some(new_state))` — applied successfully.
    /// - `Ok(None)` — event doesn't apply to current state (mismatch).
    /// - `Err(error)` — domain failure (e.g. arithmetic overflow).
    fn evolve(entity: &Self, event: &Self::Event) -> Result<Option<Self>, Self::Error>;

    // --- Command side: processing commands to produce events ---------

    /// Handle a command when the entity doesn't exist yet.
    ///
    /// No `&self` — impossible to accidentally reference state during
    /// creation.
    async fn initialize(
        command: Self::Command,
        services: &Self::Services,
    ) -> Result<Vec<Self::Event>, Self::Error>;

    /// Handle a command against existing state. Receives `&self`
    /// (the domain type, not `Lifecycle`), so the handler only deals
    /// with live state.
    async fn transition(
        &self,
        command: Self::Command,
        services: &Self::Services,
    ) -> Result<Vec<Self::Event>, Self::Error>;
}
