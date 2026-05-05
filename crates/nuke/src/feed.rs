//! [`Feed`] - read-only typed event streams.
//!
//! Sibling to [`crate::venue::TradingVenue`]: where TradingVenue is
//! the write side (place / check / inventory), `Feed<S>` is the read
//! side (a stream of typed events identified by a [`Subject`] `S`).
//!
//! A `Feed` impl is a transport adapter (websocket subscription,
//! REST polling loop, cqrs/es replay, in-process bus) parameterized
//! by what kind of subject it serves. A single physical venue can
//! implement both `TradingVenue` and `Feed` (e.g. a CEX that exposes
//! order updates over a websocket); other venues are read-only or
//! write-only.
//!
//! Feeds do not require credentials for *public* subjects (trades,
//! market data); private subjects (account orders, deposits,
//! withdrawals) typically do. The credential boundary is the
//! adapter's concern, not the framework's.

use std::pin::Pin;

use futures_util::Stream;

use crate::subject::Subject;

/// Boxed event stream produced by a [`Feed`].
pub type FeedStream<E, Err> = Pin<Box<dyn Stream<Item = Result<E, Err>> + Send + 'static>>;

/// A read-only typed event stream identified by a [`Subject`] `S`.
///
/// Calling [`subscribe`](Self::subscribe) returns a stream of
/// `S::Event` values produced by whatever transport the impl wraps.
/// The stream ends when the underlying source closes; transient
/// failures inside the stream surface as `Err` items.
pub trait Feed<S: Subject>: Send + Sync + 'static {
    /// Errors the feed's transport can return on individual items.
    type Error: std::error::Error + Send + Sync + 'static;

    /// Open the subscription. Each call returns a fresh stream;
    /// implementations may share an underlying connection or not at
    /// their discretion.
    fn subscribe(&self) -> FeedStream<S::Event, Self::Error>;
}
