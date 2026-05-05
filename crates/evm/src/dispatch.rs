//! Type-level walker over a reactor's [`SubjectList`] that opens one
//! `eth_subscribe` per [`Subject`] and builds a runtime [`Dispatcher`]
//! from per-subject decoder + `HasSubject::inject` closures.
//!
//! Mirrors the recursive `Cons<H, T>` traversal pattern used in
//! event-sorcery, adapted for async subscription setup.

use std::collections::HashMap;
use std::future::Future;
use std::pin::Pin;

use alloy_primitives::Address;
use nuke::{Cons, HasSubject, Nil, SubjectList};

use crate::subject::EvmSubject;
use crate::subscription::RawLog;
use crate::transport::EvmWsSource;

/// Errors produced by the EVM dispatch path.
#[derive(Debug, thiserror::Error)]
pub enum DispatchError {
    /// A log arrived for an address we don't have a registered decoder
    /// for.
    #[error("unregistered address: {0:?}")]
    UnregisteredAddress(Address),
    /// ABI decode failure.
    #[error("decode error: {0}")]
    Decode(#[from] crate::DecodeError),
}

/// Boxed decoder: takes a [`RawLog`] and yields the reactor's event
/// union (or a `DispatchError`).
type BoxedDecoder<L> = Box<
    dyn Fn(&RawLog) -> std::result::Result<<L as SubjectList>::Event, DispatchError> + Send + Sync,
>;

/// Address-keyed dispatcher from [`RawLog`] to the reactor's typed
/// event union.
pub struct Dispatcher<L: SubjectList> {
    table: HashMap<Address, BoxedDecoder<L>>,
}

impl<L: SubjectList> Dispatcher<L> {
    fn new() -> Self {
        Self {
            table: HashMap::new(),
        }
    }

    pub(crate) fn dispatch(&self, log: &RawLog) -> std::result::Result<L::Event, DispatchError> {
        self.table
            .get(&log.address)
            .ok_or(DispatchError::UnregisteredAddress(log.address))
            .and_then(|decode| decode(log))
    }
}

/// Type-level walker that opens subscriptions and builds the
/// dispatcher table.
///
/// `L` is the *target* event-union type (always the reactor's full
/// `Subjects` type — the recursive walker reuses the same `L` so each
/// step can call `HasSubject<S>::inject` for its specific `S`).
pub trait Subscribe<L: SubjectList> {
    fn open_all<'a>(
        source: &'a EvmWsSource,
    ) -> Pin<Box<dyn Future<Output = nuke::Result<Dispatcher<L>>> + Send + 'a>>;
}

impl<L: SubjectList> Subscribe<L> for Nil {
    fn open_all<'a>(
        _source: &'a EvmWsSource,
    ) -> Pin<Box<dyn Future<Output = nuke::Result<Dispatcher<L>>> + Send + 'a>> {
        Box::pin(async { Ok(Dispatcher::new()) })
    }
}

impl<H, T, L> Subscribe<L> for Cons<H, T>
where
    H: EvmSubject,
    H::Id: From<Address> + Send + Sync + 'static,
    H::Event: Send + Sync + 'static,
    T: Subscribe<L> + 'static,
    L: SubjectList + HasSubject<H> + 'static,
    L::Event: Send + 'static,
{
    fn open_all<'a>(
        source: &'a EvmWsSource,
    ) -> Pin<Box<dyn Future<Output = nuke::Result<Dispatcher<L>>> + Send + 'a>> {
        Box::pin(async move {
            source.subscribe(H::subscription()).await?;
            let mut dispatcher = T::open_all(source).await?;
            dispatcher.table.insert(
                H::address(),
                Box::new(|log: &RawLog| {
                    let id = H::Id::from(log.address);
                    let event = H::decode(log).map_err(DispatchError::from)?;
                    Ok(<L as HasSubject<H>>::inject(id, event))
                }),
            );
            Ok(dispatcher)
        })
    }
}
