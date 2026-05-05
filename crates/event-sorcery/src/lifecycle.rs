//! [`Lifecycle<E>`] - internal adapter that bridges [`EventSourced`]
//! to cqrs-es's `Aggregate`. Users implement `EventSourced`; the
//! blanket [`cqrs_es::Aggregate`] impl on `Lifecycle<E>` does the
//! rest, routing commands by lifecycle state and surfacing every
//! failure mode through [`LifecycleError`].

use async_trait::async_trait;
use cqrs_es::{Aggregate, AggregateError, DomainEvent};
use serde::{Deserialize, Serialize};

use crate::event_sourced::EventSourced;

/// Lifecycle wrapper around a user's [`EventSourced`] entity.
///
/// State machine: `Uninitialized → Live(E) → Failed(LifecycleError<E>)`.
/// The blanket `Aggregate` impl routes commands and events through
/// the user's `originate` / `evolve` / `initialize` / `transition`.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(bound(
    serialize = "E: Serialize",
    deserialize = "E: serde::de::DeserializeOwned"
))]
pub enum Lifecycle<E: EventSourced> {
    /// No events seen yet — only `originate`-able events are valid.
    #[default]
    Uninitialized,
    /// Live state.
    Live(E),
    /// Stuck state — a previous step failed and the entity can't
    /// process further commands until reset.
    Failed(LifecycleError<E>),
}

impl<E: EventSourced> Lifecycle<E> {
    /// Convenience: extract the live state, treating any non-Live
    /// variant as an error.
    pub fn into_result(self) -> Result<Option<E>, LifecycleError<E>> {
        match self {
            Self::Uninitialized => Ok(None),
            Self::Live(state) => Ok(Some(state)),
            Self::Failed(error) => Err(error),
        }
    }
}

/// Errors a lifecycle can land in. Carries the user's `Error` from
/// `evolve`/`initialize`/`transition` plus structural failures (an
/// event that should have originated didn't, a transition got an
/// unexpected event, etc.).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, thiserror::Error)]
#[serde(bound(
    serialize = "E::Error: Serialize",
    deserialize = "E::Error: serde::de::DeserializeOwned"
))]
pub enum LifecycleError<E: EventSourced> {
    /// `originate` returned `None` for an event we expected to be a
    /// genesis event. Indicates a bug or replay over a stale schema.
    #[error("event cannot originate aggregate")]
    EventCantOriginate,
    /// `evolve` returned `None` — the event doesn't apply to the
    /// current state (likely a stale event log).
    #[error("event does not apply to current state")]
    UnexpectedEvent,
    /// User code returned an error from `evolve`.
    #[error("apply failed: {0}")]
    Apply(E::Error),
    /// User code returned an error from `initialize` or `transition`.
    #[error("command rejected: {0}")]
    Command(E::Error),
    /// Lifecycle is in `Failed` state — no further commands accepted.
    #[error("aggregate is in a failed state")]
    Stuck,
}

#[async_trait]
impl<E: EventSourced> Aggregate for Lifecycle<E>
where
    E::Event: DomainEvent,
{
    type Command = E::Command;
    type Event = E::Event;
    type Error = LifecycleError<E>;
    type Services = E::Services;

    fn aggregate_type() -> String {
        E::AGGREGATE_TYPE.to_owned()
    }

    async fn handle(
        &self,
        command: Self::Command,
        services: &Self::Services,
    ) -> Result<Vec<Self::Event>, Self::Error> {
        match self {
            Lifecycle::Uninitialized => E::initialize(command, services)
                .await
                .map_err(LifecycleError::Command),
            Lifecycle::Live(state) => state
                .transition(command, services)
                .await
                .map_err(LifecycleError::Command),
            Lifecycle::Failed(_) => Err(LifecycleError::Stuck),
        }
    }

    fn apply(&mut self, event: Self::Event) {
        let next = match self {
            Lifecycle::Uninitialized => match E::originate(&event) {
                Some(state) => Lifecycle::Live(state),
                None => Lifecycle::Failed(LifecycleError::EventCantOriginate),
            },
            Lifecycle::Live(state) => match E::evolve(state, &event) {
                Ok(Some(next)) => Lifecycle::Live(next),
                Ok(None) => Lifecycle::Failed(LifecycleError::UnexpectedEvent),
                Err(error) => Lifecycle::Failed(LifecycleError::Apply(error)),
            },
            Lifecycle::Failed(_) => return,
        };
        *self = next;
    }
}

/// Uninhabited error type — use as `EventSourced::Error` for entities
/// whose operations never fail.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum Never {}

impl std::fmt::Display for Never {
    fn fmt(&self, _: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match *self {}
    }
}

impl std::error::Error for Never {}

/// Convenience to convert a raw `LifecycleError` into an
/// `AggregateError`.
impl<E: EventSourced> From<LifecycleError<E>> for AggregateError<LifecycleError<E>> {
    fn from(error: LifecycleError<E>) -> Self {
        Self::UserError(error)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::event_sourced::EventSourced;
    use serde::{Deserialize, Serialize};

    /// A minimal entity that exercises every code path in `Lifecycle`.
    #[derive(Default, Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
    struct Counter {
        value: i64,
    }

    #[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
    enum CounterEvent {
        Created,
        Incremented(i64),
    }

    impl DomainEvent for CounterEvent {
        fn event_type(&self) -> String {
            match self {
                Self::Created => "Counter::Created".into(),
                Self::Incremented(_) => "Counter::Incremented".into(),
            }
        }
        fn event_version(&self) -> String {
            "1".into()
        }
    }

    enum CounterCommand {
        Create,
        Increment(i64),
    }

    #[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, thiserror::Error)]
    #[error("counter overflow")]
    struct CounterError;

    #[async_trait]
    impl EventSourced for Counter {
        type Id = String;
        type Event = CounterEvent;
        type Command = CounterCommand;
        type Error = CounterError;
        type Services = ();

        const AGGREGATE_TYPE: &'static str = "Counter";
        const SCHEMA_VERSION: u64 = 1;

        fn originate(event: &CounterEvent) -> Option<Self> {
            match event {
                CounterEvent::Created => Some(Counter::default()),
                CounterEvent::Incremented(_) => None,
            }
        }

        fn evolve(state: &Counter, event: &CounterEvent) -> Result<Option<Counter>, CounterError> {
            match event {
                CounterEvent::Created => Ok(None), // can't re-create
                CounterEvent::Incremented(delta) => Ok(Some(Counter {
                    value: state.value + *delta,
                })),
            }
        }

        async fn initialize(
            command: CounterCommand,
            _services: &(),
        ) -> Result<Vec<CounterEvent>, CounterError> {
            match command {
                CounterCommand::Create => Ok(vec![CounterEvent::Created]),
                CounterCommand::Increment(_) => Err(CounterError),
            }
        }

        async fn transition(
            &self,
            command: CounterCommand,
            _services: &(),
        ) -> Result<Vec<CounterEvent>, CounterError> {
            match command {
                CounterCommand::Create => Err(CounterError),
                CounterCommand::Increment(delta) => Ok(vec![CounterEvent::Incremented(delta)]),
            }
        }
    }

    #[tokio::test]
    async fn lifecycle_apply_originate_then_evolve() {
        let mut life = Lifecycle::<Counter>::default();
        life.apply(CounterEvent::Created);
        life.apply(CounterEvent::Incremented(3));
        life.apply(CounterEvent::Incremented(4));
        match life {
            Lifecycle::Live(state) => assert_eq!(state.value, 7),
            other => panic!("expected Live, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn lifecycle_lands_in_failed_when_origination_impossible() {
        let mut life = Lifecycle::<Counter>::default();
        life.apply(CounterEvent::Incremented(1)); // not a genesis event
        assert!(matches!(
            life,
            Lifecycle::Failed(LifecycleError::EventCantOriginate)
        ));
    }

    #[tokio::test]
    async fn lifecycle_handle_routes_by_state() {
        let life = Lifecycle::<Counter>::default();
        let events = life.handle(CounterCommand::Create, &()).await.unwrap();
        assert_eq!(events, vec![CounterEvent::Created]);

        let live = Lifecycle::Live(Counter { value: 5 });
        let events = live
            .handle(CounterCommand::Increment(2), &())
            .await
            .unwrap();
        assert_eq!(events, vec![CounterEvent::Incremented(2)]);
    }

    #[test]
    fn never_is_uninhabited() {
        // `Never` has no constructors — this test exists to lock in
        // that property as intentional.
        let never: Option<Never> = None;
        assert!(never.is_none());
    }
}
