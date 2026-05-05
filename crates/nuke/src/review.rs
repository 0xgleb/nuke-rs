//! Review-and-filter primitives borrowed from `barter-rs`.
//!
//! Two trait shapes plus two typed wrappers:
//!
//! - [`Validator<T>`] - check whether a value passes a typed gate.
//!   Returns either [`Approved<T>`] (the value, wrapped in a marker
//!   that downstream code can demand) or [`Refused<T, R>`] (the same
//!   value plus a typed reason).
//! - [`Processor<E>`] - consume an event of type `E` and produce
//!   side-effects via `&mut self`. The pair of `Validator` +
//!   `Processor` lets a pipeline statically separate "checked X" from
//!   "anything claiming to be X" (and "rejected X" from "X with a
//!   reason explaining why").
//!
//! These are deliberately small. Adopters compose them - a risk
//! manager is `impl Validator<OrderRequest, Reason = RiskReason>`; a
//! reactor's [`crate::Job`] for a Buy verb is `impl
//! Processor<Approved<OrderRequest>>`.

use std::fmt::Debug;

/// Decide whether a typed value passes a gate. Implementors return
/// `Ok(())` on success or `Err(reason)` on failure; the framework's
/// [`validate`] free function turns that decision into the typed
/// [`Approved`] / [`Refused`] wrappers (so [`Approved::new`] stays
/// private to this module - a downstream step asking for
/// `Approved<T>` is guaranteed to have come through a `Validator`).
pub trait Validator<T>: Send + Sync {
    /// Typed reason returned on rejection. Implementors choose a
    /// domain-specific enum (e.g. `RiskReason`, `ComplianceReason`)
    /// so the rejection is self-explanatory and machine-routable.
    type Reason: Debug + Send + Sync;

    /// Run the gate. Borrow the input so the framework can hand the
    /// owned value to one of the [`Approved`] / [`Refused`] wrappers
    /// without forcing the implementor to give it back.
    fn check(&self, input: &T) -> Result<(), Self::Reason>;
}

/// Run a [`Validator`] against an owned value, packaging the verdict
/// into the typed [`Approved`] / [`Refused`] wrapper a downstream
/// pipeline expects.
pub fn validate<V, T>(validator: &V, input: T) -> Result<Approved<T>, Refused<T, V::Reason>>
where
    V: Validator<T>,
{
    match validator.check(&input) {
        Ok(()) => Ok(Approved::new(input)),
        Err(reason) => Err(Refused::new(input, reason)),
    }
}

/// Consume an event of type `E`. Pipelines string `Processor` impls
/// together to model their work as a chain of typed event handlers
/// (vs. one giant match).
pub trait Processor<E>: Send + Sync {
    /// Output produced after handling the event. Use `()` if the
    /// processor's purpose is purely side-effects via `&mut self`.
    type Output;

    /// Process one event.
    fn process(&mut self, event: E) -> Self::Output;
}

/// A value that has cleared a [`Validator`]. Construction is private
/// to this module so the marker is meaningful: a downstream step
/// asking for `Approved<T>` is guaranteed to have received a value
/// that passed *some* gate.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Approved<T>(T);

impl<T> Approved<T> {
    /// Unwrap the inner value. Used by downstream consumers that
    /// demand typed-approved input.
    pub fn into_inner(self) -> T {
        self.0
    }

    /// Borrow the inner value.
    pub fn as_inner(&self) -> &T {
        &self.0
    }
}

/// A value that was rejected by a [`Validator`], paired with the
/// typed reason. Carries the same `T` as a successful approval so a
/// caller can salvage / log / re-route the input.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Refused<T, R> {
    pub input: T,
    pub reason: R,
}

impl<T, R> Refused<T, R> {
    pub fn new(input: T, reason: R) -> Self {
        Self { input, reason }
    }
}

/// Sealed constructor for [`Approved`]. Only `nuke::review` callers
/// (and crates with explicit `pub use` of [`Validator`]) can build
/// approvals - that's how the marker preserves its meaning.
impl<T> Approved<T> {
    pub(crate) fn new(value: T) -> Self {
        Self(value)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Debug, PartialEq, Eq)]
    enum SizeReason {
        TooLarge { max: u32 },
    }

    struct MaxSize {
        max: u32,
    }

    impl Validator<u32> for MaxSize {
        type Reason = SizeReason;
        fn check(&self, value: &u32) -> Result<(), SizeReason> {
            if *value > self.max {
                Err(SizeReason::TooLarge { max: self.max })
            } else {
                Ok(())
            }
        }
    }

    #[test]
    fn validator_approves_in_range() {
        let v = MaxSize { max: 100 };
        let approved = validate(&v, 42).expect("under max");
        assert_eq!(approved.into_inner(), 42);
    }

    #[test]
    fn validator_refuses_over_range_with_typed_reason() {
        let v = MaxSize { max: 100 };
        let refused = validate(&v, 200).expect_err("over max");
        assert_eq!(refused.input, 200);
        assert_eq!(refused.reason, SizeReason::TooLarge { max: 100 });
    }

    /// Counter that increments per processed event. Demonstrates a
    /// stateful `Processor` impl.
    #[derive(Default)]
    struct Counter {
        seen: usize,
    }

    impl Processor<u32> for Counter {
        type Output = usize;
        fn process(&mut self, _: u32) -> usize {
            self.seen += 1;
            self.seen
        }
    }

    #[test]
    fn processor_threads_state_through_calls() {
        let mut c = Counter::default();
        assert_eq!(c.process(10), 1);
        assert_eq!(c.process(20), 2);
        assert_eq!(c.process(30), 3);
    }
}
