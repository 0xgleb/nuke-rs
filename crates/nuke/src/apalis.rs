//! Internal adapter that drives a [`Reactor`] through apalis 1.x.
//!
//! Public API: [`pump_through_apalis`] — venue-agnostic. Takes a stream
//! of typed events (the producer side is the venue's responsibility —
//! e.g. `evm::pump` builds the stream from raw chain logs) and runs an
//! apalis worker that hands each task to `reactor.react(...)`.
//!
//! The seam is intentional: switching the in-memory dequeue for a
//! persistent backend (`apalis-sql`, `apalis-redis`) or composing
//! reactions into a multi-step `apalis_workflow::Workflow` /
//! `DagFlow` is a drop-in change here, not a user-visible one.

use std::sync::Arc;
use std::time::Duration;

use apalis::prelude::{BoxDynError, PipeExt, WorkerBuilder};
use apalis_core::backend::dequeue;
use futures_util::Stream;

use crate::error::{Error, Result};
use crate::reactor::Reactor;
use crate::subscribed::SubjectList;

/// Drive `reactor` to convergence: consume `events` (typed event union
/// matching the reactor's subject list), push each onto an apalis
/// `dequeue` backend through `PipeExt::pipe_to`, and run an apalis
/// `Worker` that hands each task to `reactor.react(...)`.
///
/// Venue-agnostic — the producer of the event stream is the adapter's
/// concern (see e.g. `evm::pump` in the EVM adapter crate).
pub async fn pump_through_apalis<R, S>(events: S, reactor: Arc<R>) -> Result<()>
where
    R: Reactor + 'static,
    <R::Subjects as SubjectList>::Event: Clone + Send + Sync + 'static,
    S: Stream<Item = std::result::Result<<R::Subjects as SubjectList>::Event, PipelineError>>
        + Send
        + Unpin
        + 'static,
{
    // Poll interval is the *empty-queue* backoff, not per-task latency:
    // when nothing is queued, the worker sleeps this long before checking
    // again. Tasks pushed via the pipe wake the worker promptly.
    let in_memory =
        dequeue::backend::<<R::Subjects as SubjectList>::Event>(Duration::from_millis(10));
    let backend = events.pipe_to(in_memory);

    let reactor_for_handler = Arc::clone(&reactor);
    let handler = move |event: <R::Subjects as SubjectList>::Event| {
        let reactor = Arc::clone(&reactor_for_handler);
        async move {
            reactor
                .react(event)
                .await
                .map_err(|error| Box::new(error) as BoxDynError)
        }
    };

    WorkerBuilder::new("nuke-reactor")
        .backend(backend)
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
