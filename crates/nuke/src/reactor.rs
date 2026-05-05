//! The [`Reactor`] trait - what reacts to events from a list of
//! deps by *enqueuing jobs* (which apalis runs durably with
//! retries).
//!
//! Reactors are pure deciders. They observe events and decide what
//! work to enqueue; they never perform the work themselves. All
//! external-service side-effects (POST / signed-tx /
//! `TradingVenue::place_trade` / etc.) happen inside [`Job`] impls so
//! durability + retries + backoff are free.
//!
//! The event type is *computed* from `Dependent::Deps`, not declared
//! by hand. Implement `react` with the `.on(...).on(...).exhaustive()`
//! chain - forgetting a dep is a compile error.

use async_trait::async_trait;
use std::sync::Arc;

use crate::job::Job;
use event_sorcery::{DepList, Dependent};

/// Event reactor with exhaustive compile-time-checked handling.
///
/// ```ignore
/// deps!(ArbBot, [UniV2WethUsdc, SushiV2WethUsdc]);
///
/// #[async_trait]
/// impl Reactor for ArbBot {
///     type Job = ArbJob;
///     type Ctx = ArbCtx;
///
///     async fn react(
///         &self,
///         event: <Self::Deps as DepList>::Event,
///     ) -> Vec<ArbJob> {
///         event
///             .on(|id, sync| async move { self.on_univ2(id, sync).await })
///             .on(|id, sync| async move { self.on_sushi(id, sync).await })
///             .exhaustive()
///             .await
///     }
/// }
/// ```
#[async_trait]
pub trait Reactor: Dependent + Send + Sync {
    /// The job type emitted by this reactor's `react`. Usually an
    /// enum of every job variant the reactor produces; `Job<Ctx>` is
    /// implemented on the enum and dispatches to per-variant
    /// `perform` impls.
    type Job: Job<Self::Ctx> + Clone + Sync;

    /// The shared context the framework passes to `Job::perform`
    /// via the apalis `Data<Arc<Ctx>>` extractor. Bundles every
    /// `TradingVenue` impl, persistence handle, config, etc. that
    /// jobs need.
    type Ctx: Send + Sync + 'static;

    /// Decide which jobs to enqueue in response to `event`. May
    /// return zero or more. Reactors do not perform work directly;
    /// `Job::perform` does.
    async fn react(&self, event: <Self::Deps as DepList>::Event) -> Vec<Self::Job>;
}

#[async_trait]
impl<R> Reactor for Arc<R>
where
    R: Reactor,
{
    type Job = R::Job;
    type Ctx = R::Ctx;

    async fn react(&self, event: <Self::Deps as DepList>::Event) -> Vec<Self::Job> {
        R::react(self, event).await
    }
}
