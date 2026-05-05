//! EVM Source / TradingVenue adapter for the `nuke` framework.
//!
//! This crate is **not** part of the framework — it's a worked
//! implementation of `nuke::Subject` + (eventually) the abstract
//! `Source` / `TradingVenue` traits, against an Ethereum JSON-RPC
//! websocket transport. Examples and applications opt in by depending
//! on this crate; framework crates (`nuke`, `nuke-derive`) never do.
//!
//! # Layout
//!
//! - [`EvmWsSource`] — owns one ws connection + background task that
//!   does JSON-RPC framing and `eth_subscribe` management. Notifications
//!   are forwarded as [`RawLog`] values into an `mpsc` channel.
//! - [`Subscribe`] — type-level walker that opens one `eth_subscribe`
//!   per `Subject` in a reactor's list and builds a [`Dispatcher`].
//! - [`pump`] — wires `EvmWsSource → Subscribe → Dispatcher` and feeds
//!   the resulting typed event stream into the framework's
//!   `nuke::pump_through_apalis` so apalis owns the run loop.

mod dispatch;
mod subject;
mod subscription;
mod transport;

pub use dispatch::{Dispatcher, Subscribe};
// Both the trait and the derive macro are exported as `EvmSubject` —
// they live in different namespaces (type vs macro) so Rust resolves
// `impl EvmSubject for ...` (trait) and `#[derive(EvmSubject)]` (macro)
// without ambiguity.
pub use evm_derive::EvmSubject;
pub use subject::EvmSubject;
pub use subscription::{RawLog, SubscriptionSpec};
pub use transport::EvmWsSource;

use std::sync::Arc;

use futures_util::StreamExt;
use nuke::{Reactor, SubjectList, pump_through_apalis};
use tokio_stream::wrappers::ReceiverStream;

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

/// Re-exports used by the `#[derive(EvmSubject)]` proc-macro
/// expansion. Not part of the supported public API.
#[doc(hidden)]
pub mod reexports {
    pub use alloy_primitives;
    pub use alloy_sol_types;
}

/// Run a reactor against an EVM JSON-RPC websocket source.
///
/// Reads `R::Subjects` at compile time, opens the corresponding
/// `eth_subscribe` calls on `source`, and pipes the resulting typed
/// event stream through `nuke::pump_through_apalis`, which invokes
/// `reactor.react(event)` per item, enqueues the resulting jobs, and
/// runs them via `Job::perform(&ctx)`. The framework owns the apalis
/// run loop; this function is the EVM-specific bridge.
pub async fn pump<R>(mut source: EvmWsSource, reactor: Arc<R>, ctx: Arc<R::Ctx>) -> nuke::Result<()>
where
    R: Reactor + 'static,
    R::Subjects: Subscribe<R::Subjects>,
    R::Ctx: Send + Sync + 'static,
    <R::Subjects as SubjectList>::Event: Send + 'static,
{
    let dispatcher = <R::Subjects as Subscribe<R::Subjects>>::open_all(&source).await?;
    let log_stream = source
        .take_log_stream()
        .ok_or_else(|| nuke::Error::msg("EvmWsSource log stream already taken"))?;

    let dispatcher = Arc::new(dispatcher);
    let events = ReceiverStream::new(log_stream).map({
        let dispatcher = Arc::clone(&dispatcher);
        move |log| {
            dispatcher
                .dispatch(&log)
                .map_err(|error| nuke::PipelineError::new(error.to_string()))
        }
    });

    pump_through_apalis(events, reactor, ctx).await
}
