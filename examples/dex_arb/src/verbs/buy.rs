//! `Buy` verb: submit an order to a venue.

use std::sync::Arc;
use std::time::Duration;

use apalis_workflow::DagFlow;
use apalis_workflow::dag::NodeHandle;
use evm::EvmChain;
use nuke::policy::{Action, DecisionTag};
use nuke::{Order, OrderId, OrderRequest, TradingVenue};

use super::OrderResult;
use super::submit::lower_submit;

/// Buy verb: submit an order to a venue. Generic over any
/// [`TradingVenue<L>`] whose value-object types match the
/// framework's [`OrderRequest`] / [`OrderId`] / [`Order`] vocabulary.
///
/// `place_timeout` bounds the `venue.place_trade` call so a stalled
/// venue cannot hang the reactor; the lowered submit node converts an
/// elapsed timeout into a typed [`OrderResult::Failed`] with a
/// `"timeout: ..."`-prefixed error string.
///
/// In a strategy: `policy! { given [spread.lt(zero)] then do
/// Buy { qty, instrument, venue, place_timeout } }` (modulo the macro
/// shape).
pub struct Buy<V>
where
    V: TradingVenue<EvmChain, OrderRequest = OrderRequest, OrderId = OrderId, Order = Order>,
{
    pub request: OrderRequest,
    pub venue: Arc<V>,
    pub place_timeout: Duration,
}

// Manual Clone / Debug so the derives don't drag spurious `V: Clone`
// / `V: Debug` bounds in: we only ever hold `Arc<V>` (always Clone)
// and the Debug impl deliberately omits venue contents.
impl<V> Clone for Buy<V>
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

impl<V> std::fmt::Debug for Buy<V>
where
    V: TradingVenue<EvmChain, OrderRequest = OrderRequest, OrderId = OrderId, Order = Order>,
{
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Buy")
            .field("request", &self.request)
            .field("venue", &"<Arc<V>>")
            .field("place_timeout", &self.place_timeout)
            .finish()
    }
}

impl<V> Action for Buy<V>
where
    V: TradingVenue<EvmChain, OrderRequest = OrderRequest, OrderId = OrderId, Order = Order>,
    V: Send + Sync + 'static,
{
    const KIND: &'static str = "verb.buy";
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
            "verb.buy/submit",
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
    use nuke::OrderState;
    use nuke::domain::Qty;

    #[test]
    fn buy_carries_request_and_venue_and_kind() {
        let buy: Buy<EvmRpcVenue> = Buy {
            request: sample_request(),
            venue: sample_venue(),
            place_timeout: sample_timeout(),
        };
        assert_eq!(<Buy<EvmRpcVenue> as Action>::KIND, "verb.buy");
        assert_eq!(buy.request.qty, Qty::new(d(1)));
        // Order::id smoke-test: the verb's TradingVenue uses the
        // framework's Order vocabulary.
        let _id_borrow: &OrderId = nuke::Identifier::id(&Order {
            id: OrderId::new("STUB"),
            request: sample_request(),
            state: OrderState::Open,
            filled: Qty::new(d(0)),
        });
    }
}
