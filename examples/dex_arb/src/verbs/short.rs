//! `Short` verb: open a short position. v0 uses the same payload
//! shape as [`Buy`](super::Buy) / [`Sell`](super::Sell) -
//! margin-aware venues will refine the request type when they're
//! wired.

use std::sync::Arc;
use std::time::Duration;

use apalis_workflow::DagFlow;
use apalis_workflow::dag::NodeHandle;
use evm::EvmChain;
use nuke::policy::{Action, DecisionTag};
use nuke::{Order, OrderId, OrderRequest, TradingVenue};

use super::OrderResult;
use super::submit::lower_submit;

/// Short verb: open a short position. v0 uses the same payload
/// shape as [`Buy`](super::Buy) / [`Sell`](super::Sell) -
/// margin-aware venues will refine the request type when they're
/// wired.
pub struct Short<V>
where
    V: TradingVenue<EvmChain, OrderRequest = OrderRequest, OrderId = OrderId, Order = Order>,
{
    pub request: OrderRequest,
    pub venue: Arc<V>,
    pub place_timeout: Duration,
}

impl<V> Clone for Short<V>
where
    V: TradingVenue<EvmChain, OrderRequest = OrderRequest, OrderId = OrderId, Order = Order>,
{
    fn clone(&self) -> Self {
        Self {
            request: self.request.clone(),
            venue: Arc::clone(&self.venue),
            place_timeout: self.place_timeout,
        }
    }
}

impl<V> std::fmt::Debug for Short<V>
where
    V: TradingVenue<EvmChain, OrderRequest = OrderRequest, OrderId = OrderId, Order = Order>,
{
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Short")
            .field("request", &self.request)
            .field("venue", &"<Arc<V>>")
            .field("place_timeout", &self.place_timeout)
            .finish()
    }
}

impl<V> Action for Short<V>
where
    V: TradingVenue<EvmChain, OrderRequest = OrderRequest, OrderId = OrderId, Order = Order>,
    V: Send + Sync + 'static,
{
    const KIND: &'static str = "verb.short";
    type Input = DecisionTag;
    type Output = OrderResult;

    fn lower<B, Err>(
        &self,
        dag: &DagFlow<B>,
        gate: &nuke::policy::PolicyGate<'_, B>,
    ) -> NodeHandle<Self::Input, Self::Output>
    where
        B: nuke::policy::LowerBackend<Self::Input, Self::Output, Err>,
        Err: Into<apalis_core::error::BoxDynError> + Send + 'static,
    {
        lower_submit(
            dag,
            gate,
            "verb.short/submit",
            Arc::clone(&self.venue),
            self.request.clone(),
            self.place_timeout,
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::verbs::test_support::{d, sample_request, sample_timeout, sample_venue};

    use evm::EvmRpcVenue;
    use nuke::domain::Qty;

    #[test]
    fn short_kind_is_distinct_from_buy_and_sell() {
        let short: Short<EvmRpcVenue> = Short {
            request: sample_request(),
            venue: sample_venue(),
            place_timeout: sample_timeout(),
        };
        assert_eq!(<Short<EvmRpcVenue> as Action>::KIND, "verb.short");
        assert_eq!(short.request.qty, Qty::new(d(1)));
    }
}
