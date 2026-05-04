//! Internal adapter that drives a [`Reactor`] through apalis 1.x.
//!
//! The user implements `Reactor`. Internally, every decoded event flows
//! through an apalis [`dequeue`-backed in-memory backend][dequeue::backend]
//! via the [`PipeExt`] adapter, and is consumed by a [`WorkerBuilder`]-built
//! worker that calls `reactor.react(...)` per task. The public API never
//! names apalis types — `nuke::run` returns a future and that's it.
//!
//! The seam is intentional: switching the in-memory dequeue for a
//! persistent backend (`apalis-sql`, `apalis-redis`) or composing
//! reactions into a multi-step `apalis_workflow::Workflow` /
//! `DagFlow` is a drop-in change here, not a user-visible one.

use std::sync::Arc;
use std::time::Duration;

use apalis::prelude::{BoxDynError, PipeExt, WorkerBuilder};
use apalis_core::backend::dequeue;
use futures_util::StreamExt;
use tokio::sync::mpsc;
use tokio_stream::wrappers::ReceiverStream;

use crate::error::{Error, Result};
use crate::evm::{Dispatcher, RawLog};
use crate::reactor::Reactor;
use crate::subscribed::SubjectList;

/// Drive `reactor` to convergence: pull `RawLog`s from `log_stream`,
/// dispatch each into the reactor's typed event union, push the result
/// onto an apalis `dequeue` backend through `PipeExt::pipe_to`, and run
/// an apalis `Worker` that hands each task to `reactor.react(...)`.
pub(crate) async fn pump_through_apalis<R>(
    log_stream: mpsc::Receiver<RawLog>,
    dispatcher: Dispatcher<R::Subjects>,
    reactor: Arc<R>,
) -> Result<()>
where
    R: Reactor + 'static,
    <R::Subjects as SubjectList>::Event: Clone + Send + Sync + 'static,
{
    let dispatcher = Arc::new(dispatcher);

    let event_stream = ReceiverStream::new(log_stream).map({
        let dispatcher = Arc::clone(&dispatcher);
        move |log| {
            dispatcher
                .dispatch(&log)
                .map_err(|error| ReactorPipelineError(error.to_string()))
        }
    });

    // Poll interval is the *empty-queue* backoff, not per-task latency:
    // when nothing is queued, the worker sleeps this long before checking
    // again. Tasks pushed via the pipe wake the worker promptly.
    let in_memory =
        dequeue::backend::<<R::Subjects as SubjectList>::Event>(Duration::from_millis(10));
    let backend = event_stream.pipe_to(in_memory);

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

/// Tiny sized error type so the dispatch result can satisfy the
/// `Stream<Item = Result<_, E: std::error::Error + Send + Sync>>` bound
/// `PipeExt` requires. We log the underlying chain via `Display` and
/// surface it through the worker's transport-error path.
#[derive(Debug, thiserror::Error)]
#[error("nuke pipeline error: {0}")]
struct ReactorPipelineError(String);
