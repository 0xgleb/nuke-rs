//! Dispatcher (address-keyed decoder table) and [`Wire`] / [`Transport`]
//! impls that plug `EvmWsSource` into the framework's generic
//! [`nuke::Subscribe`] walker.
//!
//! No type-level recursion lives here - that's
//! [`nuke::subscribe`](nuke::subscribe). This crate supplies only the
//! per-dep wiring step (open one `eth_subscribe`, register one
//! decoder).

use std::collections::HashMap;
use std::future::Future;
use std::pin::Pin;

use alloy_primitives::Address;
use nuke::{DepList, HasDep, Transport, Wire};

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
type BoxedDecoder<L> =
    Box<dyn Fn(&RawLog) -> std::result::Result<<L as DepList>::Event, DispatchError> + Send + Sync>;

/// Address-keyed dispatcher from [`RawLog`] to the reactor's typed
/// event union.
pub struct Dispatcher<L: DepList> {
    table: HashMap<Address, BoxedDecoder<L>>,
}

impl<L: DepList> Default for Dispatcher<L> {
    fn default() -> Self {
        Self {
            table: HashMap::new(),
        }
    }
}

impl<L: DepList> Dispatcher<L> {
    pub(crate) fn dispatch(&self, log: &RawLog) -> std::result::Result<L::Event, DispatchError> {
        self.table
            .get(&log.address)
            .ok_or(DispatchError::UnregisteredAddress(log.address))
            .and_then(|decode| decode(log))
    }
}

impl Transport for EvmWsSource {
    type Out<L: DepList + 'static> = Dispatcher<L>;
}

impl<H> Wire<H> for EvmWsSource
where
    H: EvmSubject,
    H::Id: From<Address> + Send + Sync + 'static,
    H::Event: Send + Sync + 'static,
{
    fn wire<'a, L>(
        &'a self,
        out: &'a mut Dispatcher<L>,
    ) -> Pin<Box<dyn Future<Output = nuke::Result<()>> + Send + 'a>>
    where
        L: DepList + HasDep<H> + 'static,
    {
        Box::pin(async move {
            self.subscribe(H::subscription()).await?;
            out.table.insert(
                H::address(),
                Box::new(|log: &RawLog| {
                    let id = H::Id::from(log.address);
                    let event = H::decode(log).map_err(DispatchError::from)?;
                    Ok(<L as HasDep<H>>::inject(id, event))
                }),
            );
            Ok(())
        })
    }
}
