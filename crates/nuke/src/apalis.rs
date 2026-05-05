//! Internal adapter that drives a [`Reactor`] through apalis 1.x.
//!
//! Public API: [`pump_through_apalis`] - venue-agnostic. Takes a
//! stream of typed events (the producer side is the venue's
//! responsibility - e.g. `evm::pump` builds the stream from raw chain
//! logs) plus a `Reactor` and its shared `Ctx`, and runs the
//! event-to-job DAG: `event -> reactor.react -> Vec<Job> -> apalis
//! storage -> Worker -> Job::perform(&ctx)`.
//!
//! The seam is intentional: switching the in-memory dequeue for a
//! persistent backend (`apalis-sql`, `apalis-redis`) or composing
//! multi-step `apalis_workflow::DagFlow` reactions is a drop-in
//! change here, not a user-visible one.

use std::sync::Arc;
use std::time::Duration;

use apalis::prelude::{Data, PipeExt, WorkerBuilder};
use apalis_core::backend::dequeue;
use futures_util::{Stream, StreamExt, stream};

use crate::error::{Error, Result};
use crate::job::work;
use crate::reactor::Reactor;
use crate::subscribed::DepList;

/// Drive `reactor` to convergence: consume `events` (typed event
/// union matching the reactor's subject list), invoke `reactor.react`
/// per event to obtain a `Vec<R::Job>`, push each Job onto an apalis
/// `dequeue` backend, and run an apalis `Worker` that hands each Job
/// to [`Job::perform`] (with retries via [`work`]).
///
/// `ctx` is the shared context the framework injects into every Job
/// invocation via apalis's `Data<Arc<Ctx>>` extractor. Bundle every
/// `TradingVenue` impl, persistence handle, config, etc. that any
/// Job needs.
///
/// Venue-agnostic: the producer of the event stream is the adapter's
/// concern (see e.g. `evm::pump` in the EVM adapter crate).
pub async fn pump_through_apalis<R, S>(events: S, reactor: Arc<R>, ctx: Arc<R::Ctx>) -> Result<()>
where
    R: Reactor + 'static,
    R::Ctx: Send + Sync + 'static,
    <R::Deps as DepList>::Event: Send + 'static,
    S: Stream<Item = std::result::Result<<R::Deps as DepList>::Event, PipelineError>>
        + Send
        + Unpin
        + 'static,
{
    // Transform: events -> reactor.react -> a stream of Jobs (or
    // surfaced pipeline errors).
    let job_stream = events
        .then({
            let reactor = Arc::clone(&reactor);
            move |event_result| {
                let reactor = Arc::clone(&reactor);
                async move {
                    match event_result {
                        Ok(event) => Ok(reactor.react(event).await),
                        Err(error) => Err(error),
                    }
                }
            }
        })
        .flat_map(|reaction| match reaction {
            Ok(jobs) => stream::iter(jobs.into_iter().map(Ok).collect::<Vec<_>>()),
            Err(error) => stream::iter(vec![Err(error)]),
        });

    // Poll interval is the *empty-queue* backoff. Tasks pushed via
    // the pipe wake the worker promptly.
    let in_memory = dequeue::backend::<R::Job>(Duration::from_millis(10));
    let backend = job_stream.pipe_to(in_memory);

    // Function-handler form: explicit closure so trait inference for
    // `IntoWorkerService` resolves cleanly without relying on a turbofished
    // generic free function (which can leave Args/Ctx unconstrained).
    let handler = |job: R::Job, ctx: Data<Arc<R::Ctx>>| async move {
        work::<R::Ctx, R::Job>(job, ctx).await;
    };

    WorkerBuilder::new("nuke-reactor")
        .backend(backend)
        .data(ctx)
        .build(handler)
        .run()
        .await
        .map_err(|error| Error::Transport(Box::new(error)))?;
    Ok(())
}

/// Sized error type for stream stages between a venue adapter and
/// [`pump_through_apalis`]. Adapters wrap whatever decode / framing
/// errors they have into this.
///
/// Required because `PipeExt` needs `Stream<Item = Result<_, E:
/// std::error::Error + Send + Sync>>` and we want a single stable
/// type at the framework boundary.
#[derive(Debug, thiserror::Error)]
#[error("nuke pipeline error: {0}")]
pub struct PipelineError(String);

impl PipelineError {
    pub fn new(message: impl Into<String>) -> Self {
        Self(message.into())
    }
}
