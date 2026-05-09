//! `Sell` verb: mirror of [`Buy`](super::Buy), same shape.

use std::sync::Arc;
use std::time::Duration;

use apalis_workflow::DagFlow;
use apalis_workflow::dag::NodeHandle;
use evm::EvmChain;
use nuke::policy::{Action, DecisionTag};
use nuke::{Order, OrderId, OrderRequest, TradingVenue};

use super::OrderResult;
use super::submit::lower_submit;

/// Sell verb: mirror of [`Buy`](super::Buy), same shape.
pub struct Sell<V>
where
    V: TradingVenue<EvmChain, OrderRequest = OrderRequest, OrderId = OrderId, Order = Order>,
{
    pub request: OrderRequest,
    pub venue: Arc<V>,
    pub place_timeout: Duration,
}

impl<V> Clone for Sell<V>
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

impl<V> std::fmt::Debug for Sell<V>
where
    V: TradingVenue<EvmChain, OrderRequest = OrderRequest, OrderId = OrderId, Order = Order>,
{
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Sell")
            .field("request", &self.request)
            .field("venue", &"<Arc<V>>")
            .field("place_timeout", &self.place_timeout)
            .finish()
    }
}

impl<V> Action for Sell<V>
where
    V: TradingVenue<EvmChain, OrderRequest = OrderRequest, OrderId = OrderId, Order = Order>,
    V: Send + Sync + 'static,
{
    const KIND: &'static str = "verb.sell";
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
            "verb.sell/submit",
            Arc::clone(&self.venue),
            self.request.clone(),
            self.place_timeout,
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::verbs::test_support::{sample_request, sample_timeout, sample_venue};

    use evm::EvmRpcVenue;
    use nuke::domain::Symbol;

    #[test]
    fn sell_kind_is_distinct_from_buy() {
        let sell: Sell<EvmRpcVenue> = Sell {
            request: sample_request(),
            venue: sample_venue(),
            place_timeout: sample_timeout(),
        };
        assert_eq!(<Sell<EvmRpcVenue> as Action>::KIND, "verb.sell");
        assert_eq!(sell.request.instrument, Symbol::new("WETH/USDC"));
    }
}
