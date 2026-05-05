//! Generic transmitter trait. Borrowed from barter-rs's `Tx`.
//!
//! [`Tx<Item, Error>`] decouples a producer from its concrete channel
//! type: a reactor wanting to "send an audit record somewhere"
//! takes `impl Tx<AuditRecord>` rather than a specific
//! `mpsc::Sender<AuditRecord>`, so the channel type (or substitution
//! with a metrics emitter, a file writer, a no-op sink, ...) can vary
//! per deployment without touching the reactor.

/// Send one item to the underlying channel.
///
/// Synchronous because the typical implementations - bounded mpsc
/// sender, broadcast sender, pre-allocated ringbuffer - are
/// non-blocking at this layer (they queue, and either block / drop /
/// error if the queue is full per their concrete semantics).
/// Async transmitters wrap a sync queue + a worker task that drains
/// it; the trait stays sync so it composes with non-async call sites.
pub trait Tx<Item> {
    /// Errors the transmitter can surface (queue full, peer
    /// disconnected, etc.). Implementor-defined.
    type Error: std::error::Error + Send + Sync + 'static;

    /// Send one item.
    fn send(&self, item: Item) -> Result<(), Self::Error>;
}

/// No-op transmitter: drops every item, never errors. Useful as a
/// default / placeholder when a pipeline reserves a transmitter slot
/// the operator doesn't need to populate.
#[derive(Debug, Default, Clone, Copy)]
pub struct DropTx;

impl<Item> Tx<Item> for DropTx {
    type Error = std::convert::Infallible;
    fn send(&self, _item: Item) -> Result<(), Self::Error> {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    /// Test transmitter that records every send into an internal Vec.
    struct RecordingTx<Item: Clone> {
        sent: Mutex<Vec<Item>>,
    }

    impl<Item: Clone> RecordingTx<Item> {
        fn new() -> Self {
            Self {
                sent: Mutex::new(Vec::new()),
            }
        }
        fn snapshot(&self) -> Vec<Item> {
            self.sent.lock().unwrap().clone()
        }
    }

    impl<Item: Clone> Tx<Item> for RecordingTx<Item> {
        type Error = std::convert::Infallible;
        fn send(&self, item: Item) -> Result<(), Self::Error> {
            self.sent.lock().unwrap().push(item);
            Ok(())
        }
    }

    #[test]
    fn recording_tx_captures_items_in_send_order() {
        let tx = RecordingTx::<u32>::new();
        tx.send(1).unwrap();
        tx.send(2).unwrap();
        tx.send(3).unwrap();
        assert_eq!(tx.snapshot(), vec![1, 2, 3]);
    }

    #[test]
    fn drop_tx_succeeds_silently() {
        let tx = DropTx;
        let _: Result<(), _> = Tx::<u32>::send(&tx, 42);
    }
}
