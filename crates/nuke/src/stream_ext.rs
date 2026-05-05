//! Extension methods on [`Stream`] used by reactor pipelines.
//!
//! Borrowed from barter-rs's `BarterStreamExt`. Three combinators
//! that show up over and over in event-source pipelines and that the
//! upstream `futures-util` crate doesn't ship:
//!
//! - [`StreamExt::with_index`] - tag every item with its 0-based
//!   sequence number, useful for ordering / dedup / audit.
//! - [`StreamExt::with_timeout`] - tag every item with the wall-clock
//!   timestamp it was observed at.
//! - [`StreamExt::forward_by`] - send every item through a
//!   [`crate::Tx`] before re-yielding it, so a pipeline can fan-out
//!   to an audit / metrics sink without breaking the main flow.

use std::pin::Pin;
use std::task::{Context, Poll};
use std::time::SystemTime;

use futures_util::Stream;
use pin_project_lite::pin_project;

use crate::Tx;

/// Extension trait. Blanket-impl'd on every [`Stream`].
pub trait StreamExt: Stream {
    /// Tag every item with its 0-based sequence number.
    fn with_index(self) -> WithIndex<Self>
    where
        Self: Sized,
    {
        WithIndex {
            inner: self,
            next: 0,
        }
    }

    /// Tag every item with the wall-clock timestamp it was emitted at.
    fn with_timestamp(self) -> WithTimestamp<Self>
    where
        Self: Sized,
    {
        WithTimestamp { inner: self }
    }

    /// Send every item through `tx` (cloned) before re-yielding it.
    /// Pipeline authors use this to fan a stream out to an audit /
    /// metrics sink without breaking the main consumer's flow.
    /// Errors from `tx.send` are silently dropped - the stream's
    /// primary consumer takes precedence.
    fn forward_clone_by<T>(self, tx: T) -> ForwardCloneBy<Self, T>
    where
        Self: Sized,
        Self::Item: Clone,
        T: Tx<Self::Item>,
    {
        ForwardCloneBy { inner: self, tx }
    }
}

impl<S: Stream + ?Sized> StreamExt for S {}

pin_project! {
    /// Output of [`StreamExt::with_index`].
    pub struct WithIndex<S> {
        #[pin]
        inner: S,
        next: u64,
    }
}

impl<S: Stream> Stream for WithIndex<S> {
    type Item = (u64, S::Item);
    fn poll_next(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        let this = self.project();
        match this.inner.poll_next(cx) {
            Poll::Ready(Some(item)) => {
                let index = *this.next;
                *this.next += 1;
                Poll::Ready(Some((index, item)))
            }
            Poll::Ready(None) => Poll::Ready(None),
            Poll::Pending => Poll::Pending,
        }
    }
}

pin_project! {
    /// Output of [`StreamExt::with_timestamp`].
    pub struct WithTimestamp<S> {
        #[pin]
        inner: S,
    }
}

impl<S: Stream> Stream for WithTimestamp<S> {
    type Item = (SystemTime, S::Item);
    fn poll_next(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        match self.project().inner.poll_next(cx) {
            Poll::Ready(Some(item)) => Poll::Ready(Some((SystemTime::now(), item))),
            Poll::Ready(None) => Poll::Ready(None),
            Poll::Pending => Poll::Pending,
        }
    }
}

pin_project! {
    /// Output of [`StreamExt::forward_clone_by`].
    pub struct ForwardCloneBy<S, T> {
        #[pin]
        inner: S,
        tx: T,
    }
}

impl<S, T> Stream for ForwardCloneBy<S, T>
where
    S: Stream,
    S::Item: Clone,
    T: Tx<S::Item>,
{
    type Item = S::Item;
    fn poll_next(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        let this = self.project();
        match this.inner.poll_next(cx) {
            Poll::Ready(Some(item)) => {
                // Audit / metrics fanout failures are intentionally
                // swallowed - the main consumer's flow is the
                // priority.
                let _ = this.tx.send(item.clone());
                Poll::Ready(Some(item))
            }
            Poll::Ready(None) => Poll::Ready(None),
            Poll::Pending => Poll::Pending,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use futures_util::StreamExt as _;

    #[tokio::test]
    async fn with_index_tags_sequence_starting_at_zero() {
        let items: Vec<_> = futures_util::stream::iter(['a', 'b', 'c'])
            .with_index()
            .collect()
            .await;
        assert_eq!(items, vec![(0, 'a'), (1, 'b'), (2, 'c')]);
    }

    #[tokio::test]
    async fn with_timestamp_pairs_each_item_with_a_systemtime() {
        let items: Vec<_> = futures_util::stream::iter([1u32, 2, 3])
            .with_timestamp()
            .collect()
            .await;
        assert_eq!(items.len(), 3);
        assert_eq!(items[0].1, 1);
        // Sequential SystemTime::now() reads can be equal under low
        // clock resolution, so just assert non-decreasing.
        assert!(items[0].0 <= items[1].0);
        assert!(items[1].0 <= items[2].0);
    }
}
