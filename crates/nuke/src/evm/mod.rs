//! Ethereum JSON-RPC websocket source and the type-level [`Subscribe`]
//! walker that opens one subscription per [`Subject`](crate::Subject) in
//! a reactor's list.
//!
//! The transport (`EvmWsSource`) owns the websocket connection and a
//! background task that does JSON-RPC framing + `eth_subscribe`
//! management. Notifications are forwarded as [`RawLog`] values into a
//! single `mpsc` channel; [`pump`] dispatches them by address into the
//! reactor's typed event union and calls `reactor.react(...)`.

mod dispatch;
mod subscription;
mod transport;

pub use dispatch::{Dispatcher, Subscribe};
pub use subscription::{RawLog, SubscriptionSpec};
pub use transport::EvmWsSource;

use std::sync::Arc;

use crate::apalis::pump_through_apalis;
use crate::error::{Error, Result};
use crate::reactor::Reactor;
use crate::subscribed::SubjectList;

/// ABI decode failure for an on-chain log.
#[derive(Debug, thiserror::Error)]
#[error("ABI decode error: {0}")]
pub struct DecodeError(#[source] Box<dyn std::error::Error + Send + Sync>);

impl DecodeError {
    pub fn new(error: impl std::error::Error + Send + Sync + 'static) -> Self {
        Self(Box::new(error))
    }
}

impl From<alloy_sol_types::Error> for DecodeError {
    fn from(error: alloy_sol_types::Error) -> Self {
        Self::new(error)
    }
}

/// Run loop: open subscriptions for every subject in `R::Subjects`,
/// then drive the reactor through the internal apalis adapter
/// ([`crate::apalis::pump_through_apalis`]) until the source ends or
/// an error occurs.
pub async fn pump<R>(mut source: EvmWsSource, reactor: Arc<R>) -> Result<()>
where
    R: Reactor + 'static,
    R::Subjects: Subscribe<R::Subjects>,
    <R::Subjects as SubjectList>::Event: Clone + Send + Sync + 'static,
{
    let dispatcher = <R::Subjects as Subscribe<R::Subjects>>::open_all(&source).await?;
    let log_stream = source
        .take_log_stream()
        .ok_or_else(|| Error::Config("EvmWsSource log stream already taken".into()))?;
    pump_through_apalis(log_stream, dispatcher, reactor).await
}
