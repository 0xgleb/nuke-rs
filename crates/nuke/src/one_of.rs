//! The discriminated union [`OneOf`] and its `.on(...).exhaustive()` chain.
//!
//! `Never` as the tail makes exhaustiveness a compile-time check: the
//! [`Fold::exhaustive`] method only exists when every union variant has
//! been consumed by an `.on(...)` handler.

use std::future::Future;
use std::pin::Pin;

use crate::dep::Never;

/// Discriminated union of dep events, computed from a type-level
/// dep list. See [`DepList`](crate::DepList).
#[derive(Clone)]
pub enum OneOf<Head, Tail> {
    Here(Head),
    There(Tail),
}

impl<Id, Event, Tail> OneOf<(Id, Event), Tail> {
    /// Handle the head dep in the union.
    ///
    /// Returns a [`Fold`] that you continue with `.on(...)` for each
    /// remaining dep; finish with `.exhaustive().await`.
    pub fn on<'a, T, F, Fut>(self, handler: F) -> Fold<BoxFuture<'a, T>, Tail>
    where
        F: FnOnce(Id, Event) -> Fut,
        Fut: Future<Output = T> + Send + 'a,
    {
        match self {
            Self::Here((id, event)) => Fold::Done(Box::pin(handler(id, event))),
            Self::There(tail) => Fold::Remaining(tail),
        }
    }
}

impl<A> OneOf<A, Never> {
    /// Unwrap a single-dep union.
    ///
    /// Convenience for `Deps = deps![Single]` where the computed
    /// event is `OneOf<(Id, Event), Never>`.
    pub fn into_inner(self) -> A {
        match self {
            Self::Here(inner) => inner,
            Self::There(never) => match never {},
        }
    }
}

/// Intermediate result from folding handlers over a [`OneOf`] chain.
pub enum Fold<T, Remaining> {
    Done(T),
    Remaining(Remaining),
}

type BoxFuture<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;

impl<'a, T, Id, Event, Tail> Fold<BoxFuture<'a, T>, OneOf<(Id, Event), Tail>> {
    /// Handle the next dep in the union.
    pub fn on<F, Fut>(self, handler: F) -> Fold<BoxFuture<'a, T>, Tail>
    where
        F: FnOnce(Id, Event) -> Fut,
        Fut: Future<Output = T> + Send + 'a,
    {
        match self {
            Fold::Done(fut) => Fold::Done(fut),
            Fold::Remaining(one_of) => match one_of {
                OneOf::Here((id, event)) => Fold::Done(Box::pin(handler(id, event))),
                OneOf::There(tail) => Fold::Remaining(tail),
            },
        }
    }
}

impl<T> Fold<T, Never> {
    /// Extract the result once every dep has been handled.
    ///
    /// Only available when the remaining tail is [`Never`] - i.e.,
    /// when every dep in the reactor's list has a corresponding
    /// `.on()` handler. Forgetting one is a compile error.
    pub fn exhaustive(self) -> T {
        match self {
            Self::Done(result) => result,
            Self::Remaining(never) => match never {},
        }
    }
}
