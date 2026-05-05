//! Tower-shaped adapter primitives for external event sources and
//! request/response endpoints.
//!
//! Every external thing the framework talks to is one of two shapes:
//!
//! - [`ExtStream`] - a long-running typed event source (a ws
//!   subscription, an in-process channel, a server-sent-events feed).
//!   Calling [`ExtStream::stream`] yields a `Stream` of typed events.
//! - [`ExtQuery`] - a typed request/response endpoint (a REST GET, a
//!   JSON-RPC call, a signed-tx submit, a DB SELECT). Calling
//!   [`ExtQuery::query`] runs one round-trip.
//!
//! The pair is deliberately the same shape Tower exposes at the
//! transport layer (`Service` with one async call) - adapters can
//! compose them with Tower middleware (timeout, retry, rate-limit,
//! circuit-break) without any nuke-specific glue.
//!
//! # PollingLayer
//!
//! The load-bearing piece: [`Polling`] wraps any [`ExtQuery`] plus a
//! schedule (a `Stream<Item = ()>`) and *automatically* implements
//! [`ExtStream`]. Polling adapters are not hand-written - you provide
//! a query + a tick source, you get a stream. This is what lets a
//! reactor treat "poll this REST endpoint every 5s" exactly the same
//! as "subscribe to this ws topic", at the trait level.
//!
//! ```ignore
//! struct PriceQuery { /* HTTP client, ... */ }
//! impl ExtQuery for PriceQuery { type Request = Symbol; type Response = Px; ... }
//!
//! let prices: impl ExtStream<Event = Px> = Polling {
//!     inner: PriceQuery::new(),
//!     request: Symbol::new("BTC-USD"),
//!     schedule: tokio_stream::wrappers::IntervalStream::new(
//!         tokio::time::interval(Duration::from_secs(5))
//!     ).map(|_| ()),
//! };
//! ```

use std::error::Error;
use std::future::Future;

use futures_util::{Stream, StreamExt};

/// A long-running typed external event source.
///
/// Implementations own whatever transport handle (ws connection,
/// channel receiver, ...) is needed to produce events; calling
/// [`ExtStream::stream`] consumes the value and yields a stream of
/// typed events. "Consume once" semantics keep the trait simple and
/// match the lifecycle of typical event sources.
pub trait ExtStream: Send + Sync {
    /// Decoded event type the stream yields.
    type Event: Send + 'static;
    /// Errors the stream can surface.
    type Error: Error + Send + Sync + 'static;

    /// Open the stream. Consumes `self` because most transports are
    /// single-use; if an implementor supports re-opening, expose
    /// that in their concrete API.
    fn stream(self) -> impl Stream<Item = Result<Self::Event, Self::Error>> + Send + 'static
    where
        Self: Sized;
}

/// A typed request/response endpoint.
///
/// Implementations are stateless / clonable handles around an
/// underlying transport (an HTTP client, an RPC client, ...). One
/// call = one round-trip; durability and retries belong to whatever
/// wrapper composes around the query (typically a [`crate::Job`]
/// running on apalis, or a Tower middleware stack).
pub trait ExtQuery: Send + Sync {
    /// Request payload.
    type Request: Send;
    /// Response payload.
    type Response: Send;
    /// Errors the call can surface.
    type Error: Error + Send + Sync + 'static;

    /// Issue one request. Borrows `self` so the same handle services
    /// many concurrent calls.
    fn query(
        &self,
        request: Self::Request,
    ) -> impl Future<Output = Result<Self::Response, Self::Error>> + Send;
}

/// Tower-shaped layer that turns any [`ExtQuery`] + tick stream into
/// an [`ExtStream`]. The schedule's `()` items drive the polling
/// cadence - on each tick, [`Polling::stream`] re-issues the same
/// request via the wrapped [`ExtQuery::query`] and yields the
/// response.
///
/// This is the *only* polling impl the framework ships; adopters add
/// new pollers by wrapping their `ExtQuery` and pairing it with the
/// schedule that fits their use case (interval, cron-like, externally
/// triggered).
pub struct Polling<Q, R, S> {
    pub inner: Q,
    pub request: R,
    pub schedule: S,
}

impl<Q, R, S> Polling<Q, R, S> {
    pub fn new(inner: Q, request: R, schedule: S) -> Self {
        Self {
            inner,
            request,
            schedule,
        }
    }
}

impl<Q, S> ExtStream for Polling<Q, Q::Request, S>
where
    Q: ExtQuery + Clone + 'static,
    Q::Request: Clone + Send + Sync + 'static,
    Q::Response: Send + 'static,
    S: Stream<Item = ()> + Send + Sync + 'static,
{
    type Event = Q::Response;
    type Error = Q::Error;

    fn stream(self) -> impl Stream<Item = Result<Self::Event, Self::Error>> + Send + 'static {
        let query = self.inner;
        let request = self.request;
        self.schedule.then(move |()| {
            let query = query.clone();
            let request = request.clone();
            async move { query.query(request).await }
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;
    use std::sync::atomic::{AtomicUsize, Ordering};

    #[derive(Debug, thiserror::Error)]
    #[error("infallible")]
    enum Never {}

    /// Test [`ExtQuery`]: returns `request * 2`. Counts how many times
    /// it was called so the test can verify the polling layer
    /// re-issues the request per schedule tick.
    #[derive(Clone)]
    struct Doubler {
        calls: Arc<AtomicUsize>,
    }

    impl ExtQuery for Doubler {
        type Request = u32;
        type Response = u32;
        type Error = Never;

        async fn query(&self, req: u32) -> Result<u32, Never> {
            self.calls.fetch_add(1, Ordering::SeqCst);
            Ok(req * 2)
        }
    }

    #[tokio::test]
    async fn polling_layer_auto_implements_extstream_over_extquery() {
        let calls = Arc::new(AtomicUsize::new(0));
        let polled = Polling::new(
            Doubler {
                calls: Arc::clone(&calls),
            },
            21u32,
            futures_util::stream::iter(std::iter::repeat_n((), 3)),
        );

        let outputs: Vec<u32> = polled
            .stream()
            .map(|r| r.expect("infallible query"))
            .collect()
            .await;

        assert_eq!(outputs, vec![42, 42, 42]);
        assert_eq!(calls.load(Ordering::SeqCst), 3);
    }
}
