//! Bridge per-dep [`ExtStream`]s into a [`Reactor`]'s typed dep
//! event union and run them through [`pump_through_apalis`].
//!
//! Two layers of glue:
//!
//! - [`inject_ext_stream`] - lift one [`ExtStream`] into a stream
//!   of the reactor's typed dep union ([`<L as DepList>::Event`]) by
//!   pairing each event with the dep's id and routing through
//!   [`HasDep::inject`].
//! - [`pump_dep_streams`] - fan a collection of already-injected
//!   per-dep streams into a single merged stream and feed it to
//!   [`pump_through_apalis`].
//!
//! Adapters that own a single shared transport across many deps (the
//! EVM ws case) keep using [`crate::Subscribe`] - it builds one
//! merged stream out of one transport. Adapters whose deps are each
//! their own source (REST polling, server-sent events, ...) use this
//! module instead: build per-dep [`ExtStream`]s and fan them in.

use std::pin::Pin;
use std::sync::Arc;

use futures_util::{Stream, StreamExt, stream::select_all};

use crate::apalis::{PipelineError, pump_through_apalis};
use crate::error::Result;
use crate::ext::ExtStream;
use crate::reactor::Reactor;
use event_sorcery::{Dep, DepList, HasDep};

/// Boxed dep-stream returned by [`inject_ext_stream`]. The boxing
/// lets [`pump_dep_streams`] fan in heterogeneous dep streams via
/// [`select_all`](futures_util::stream::select_all).
pub type DepStream<L> =
    Pin<Box<dyn Stream<Item = std::result::Result<<L as DepList>::Event, PipelineError>> + Send>>;

/// Lift one [`ExtStream`] producing `D::Event`s into a [`DepStream`]
/// for a reactor whose dep list `L` contains `D`.
///
/// The dep id is captured once and cloned into every emitted event.
/// Use this when one polling-or-streaming source corresponds to a
/// single dep instance (the typical REST / SSE / per-symbol ws case).
pub fn inject_ext_stream<D, L, X>(id: D::Id, ext: X) -> DepStream<L>
where
    D: Dep,
    D::Id: Clone + Send + 'static,
    L: DepList + HasDep<D> + 'static,
    X: ExtStream<Event = D::Event> + 'static,
{
    let stream = ext.stream().map(move |result| {
        result
            .map(|event| <L as HasDep<D>>::inject(id.clone(), event))
            .map_err(|error| PipelineError::new(error.to_string()))
    });
    Box::pin(stream)
}

/// Run `reactor` against a fan-in of one [`DepStream`] per dep.
///
/// Streams are merged with [`select_all`](futures_util::stream::select_all)
/// (round-robin polling); the merged stream is handed to
/// [`pump_through_apalis`] which invokes `reactor.react` per item,
/// enqueues the resulting jobs, and runs them via [`crate::Job::perform`]
/// inside an apalis worker.
///
/// This is the multi-source companion to the EVM-style single-shared-
/// transport path through [`crate::Subscribe`]. Use it when each dep
/// has its own source (per-endpoint REST polling, per-channel SSE,
/// per-symbol ws subscriptions on different endpoints, ...).
pub async fn pump_dep_streams<R>(
    streams: Vec<DepStream<R::Deps>>,
    reactor: Arc<R>,
    ctx: Arc<R::Ctx>,
) -> Result<()>
where
    R: Reactor + 'static,
    R::Ctx: Send + Sync + 'static,
    <R::Deps as DepList>::Event: Send + 'static,
{
    let merged = select_all(streams);
    pump_through_apalis(merged, reactor, ctx).await
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::reactor::Reactor;
    use async_trait::async_trait;
    use event_sorcery::{Dependent, OneOf, deps};
    use futures_util::stream;
    use serde::{Deserialize, Serialize};
    use std::sync::Mutex;

    #[derive(Debug, thiserror::Error)]
    #[error("test infallible")]
    enum NeverError {}

    /// Test [`ExtStream`]: emits its prebuilt event vector once, then
    /// closes. Each `inject_ext_stream` call below consumes one of
    /// these.
    struct PreBuilt<E> {
        events: Vec<std::result::Result<E, NeverError>>,
    }

    impl<E: Send + Sync + 'static> ExtStream for PreBuilt<E> {
        type Event = E;
        type Error = NeverError;

        fn stream(
            self,
        ) -> impl Stream<Item = std::result::Result<Self::Event, Self::Error>> + Send + 'static
        {
            stream::iter(self.events)
        }
    }

    /// Two test deps with distinct event payload types - lets us prove
    /// the union routing works (events from `Tick` land in the head
    /// arm, events from `Trade` land in the tail arm).
    struct Tick;
    struct Trade;

    #[derive(Clone, Debug, PartialEq, Eq)]
    struct TickId(u32);
    #[derive(Clone, Debug, PartialEq, Eq)]
    struct TradeId(u32);

    impl Dep for Tick {
        type Id = TickId;
        type Event = u32;
    }
    impl Dep for Trade {
        type Id = TradeId;
        type Event = String;
    }

    /// Test reactor consuming both deps - records every received event
    /// (tagged with which arm it came from) for later inspection.
    struct Recorder {
        seen: Arc<Mutex<Vec<String>>>,
    }

    deps!(Recorder, [Tick, Trade]);

    #[derive(Clone, Debug, Serialize, Deserialize)]
    struct NoOp;

    impl crate::Job<()> for NoOp {
        type Error = std::convert::Infallible;

        fn label(&self) -> crate::Label {
            crate::Label::new("noop")
        }

        async fn perform(&self, _ctx: &()) -> std::result::Result<(), Self::Error> {
            Ok(())
        }
    }

    #[async_trait]
    impl Reactor for Recorder {
        type Job = NoOp;
        type Ctx = ();

        async fn react(&self, event: <Self::Deps as DepList>::Event) -> Vec<NoOp> {
            let line = match event {
                OneOf::Here((id, value)) => format!("tick({}, {})", id.0, value),
                OneOf::There(OneOf::Here((id, value))) => format!("trade({}, {})", id.0, value),
                OneOf::There(OneOf::There(never)) => match never {},
            };
            self.seen.lock().unwrap().push(line);
            // No jobs - this test isolates the merge + reactor.react path.
            Vec::new()
        }
    }

    #[tokio::test]
    async fn merges_multiple_dep_streams_into_typed_union() {
        let seen = Arc::new(Mutex::new(Vec::<String>::new()));
        let reactor = Arc::new(Recorder {
            seen: Arc::clone(&seen),
        });

        let tick_stream: DepStream<<Recorder as Dependent>::Deps> =
            inject_ext_stream::<Tick, <Recorder as Dependent>::Deps, _>(
                TickId(7),
                PreBuilt {
                    events: vec![Ok::<u32, NeverError>(1), Ok(2)],
                },
            );
        let trade_stream: DepStream<<Recorder as Dependent>::Deps> =
            inject_ext_stream::<Trade, <Recorder as Dependent>::Deps, _>(
                TradeId(42),
                PreBuilt {
                    events: vec![Ok::<String, NeverError>("buy".into())],
                },
            );

        // We don't drive the apalis worker here (it would block forever
        // on an empty queue). We only verify the merge + injection
        // path: directly drain the merged stream and feed each event
        // to reactor.react.
        let merged = select_all(vec![tick_stream, trade_stream]);
        let collected: Vec<_> = merged.collect().await;
        assert_eq!(collected.len(), 3);
        for event_result in collected {
            let event = event_result.expect("infallible");
            let _ = reactor.react(event).await;
        }

        let mut recorded = seen.lock().unwrap().clone();
        recorded.sort();
        assert_eq!(
            recorded,
            vec![
                "tick(7, 1)".to_string(),
                "tick(7, 2)".to_string(),
                "trade(42, buy)".to_string(),
            ]
        );
    }
}
