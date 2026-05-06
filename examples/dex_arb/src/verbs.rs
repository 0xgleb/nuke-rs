//! Concrete [`Action`] verbs the policy compiler lowers into apalis
//! sub-DAGs. The framework only ships the [`Action`] trait; verbs
//! are adopter code, demonstrated here for the cross-DEX arb
//! strategy.
//!
//! Each Buy / Sell / Short verb lowers to a single-node sub-DAG
//! whose closure runs [`submit`]: it gates on the verdict and, on
//! `Allow`, calls [`TradingVenue::place_trade`] on the injected
//! `Arc<V>`, mapping the result into [`OrderResult`]
//! (`Submitted` / `Failed`) or skipping with a typed [`DecisionTag`]
//! on `Deny` / `Escalate`. [`Transfer`] follows the same shape with
//! [`TransferResult`]. The verb's first node depends on the
//! [`PolicyGate`]'s verdict task, so the venue call only fires after
//! the per-rule verdict is computed.

use std::sync::Arc;

use apalis_workflow::DagFlow;
use apalis_workflow::dag::NodeHandle;
use evm::EvmChain;
use nuke::policy::{Action, DecisionTag};
use nuke::{Ledger, Order, OrderId, OrderRequest, TradingVenue};
use serde::{Deserialize, Serialize};

/// Shared lowering for [`Buy`] / [`Sell`] / [`Short`]: add a single
/// `submit` node under the given name whose closure clones the
/// captured `venue` / `request` per invocation, runs [`submit`], and
/// depends on the verdict gate. Each verb wires its own `KIND`-keyed
/// node name (e.g. `verb.buy/submit`) so the DAG dot output stays
/// readable, but the body is identical so it lives here.
fn lower_submit<V, B, Err>(
    dag: &DagFlow<B>,
    gate: &nuke::policy::PolicyGate<'_, B>,
    node_name: &str,
    venue: Arc<V>,
    request: OrderRequest,
) -> NodeHandle<DecisionTag, OrderResult>
where
    V: TradingVenue<EvmChain, OrderRequest = OrderRequest, OrderId = OrderId, Order = Order>,
    V: Send + Sync + 'static,
    B: nuke::policy::LowerBackend<DecisionTag, OrderResult, Err>,
    Err: Into<apalis_core::error::BoxDynError> + Send + 'static,
{
    let entry = nuke::policy::action::add_node(dag, node_name, move |verdict| {
        let venue = Arc::clone(&venue);
        let request = request.clone();
        async move { submit(verdict, venue, request).await }
    });
    entry.depends_on(gate.builder())
}

/// Shared closure body for [`Buy`] / [`Sell`] / [`Short`]: gate on
/// the verdict, otherwise call `venue.place_trade(request)` and map
/// the outcome into [`OrderResult`].
async fn submit<V>(verdict: DecisionTag, venue: Arc<V>, request: OrderRequest) -> OrderResult
where
    V: TradingVenue<EvmChain, OrderRequest = OrderRequest, OrderId = OrderId, Order = Order>,
{
    let request_qty = request.qty;
    match verdict {
        // Production strategies should wrap this in `tokio::time::timeout(...)` to avoid hanging the reactor on a stalled venue.
        DecisionTag::Allow => match venue.place_trade(request).await {
            Ok(id) => OrderResult::Submitted { id, request_qty },
            Err(error) => OrderResult::Failed {
                error: format!("{error}"),
            },
        },
        DecisionTag::Deny => OrderResult::Skipped {
            verdict: DecisionTag::Deny,
        },
        DecisionTag::Escalate => OrderResult::Skipped {
            verdict: DecisionTag::Escalate,
        },
    }
}

/// Buy verb: submit an order to a venue. Generic over any
/// [`TradingVenue<L>`] whose value-object types match the
/// framework's [`OrderRequest`] / [`OrderId`] / [`Order`] vocabulary.
///
/// In a strategy: `policy! { given [spread.lt(zero)] then do
/// Buy { qty, instrument, venue } }` (modulo the macro shape).
pub struct Buy<V>
where
    V: TradingVenue<EvmChain, OrderRequest = OrderRequest, OrderId = OrderId, Order = Order>,
{
    pub request: OrderRequest,
    pub venue: Arc<V>,
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
        )
    }
}

/// Sell verb: mirror of [`Buy`], same shape.
pub struct Sell<V>
where
    V: TradingVenue<EvmChain, OrderRequest = OrderRequest, OrderId = OrderId, Order = Order>,
{
    pub request: OrderRequest,
    pub venue: Arc<V>,
}

impl<V> Clone for Sell<V>
where
    V: TradingVenue<EvmChain, OrderRequest = OrderRequest, OrderId = OrderId, Order = Order>,
{
    fn clone(&self) -> Self {
        Self {
            request: self.request.clone(),
            venue: Arc::clone(&self.venue),
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
        )
    }
}

/// Short verb: open a short position. v0 uses the same payload
/// shape as [`Buy`] / [`Sell`] - margin-aware venues will refine
/// the request type when they're wired.
pub struct Short<V>
where
    V: TradingVenue<EvmChain, OrderRequest = OrderRequest, OrderId = OrderId, Order = Order>,
{
    pub request: OrderRequest,
    pub venue: Arc<V>,
}

impl<V> Clone for Short<V>
where
    V: TradingVenue<EvmChain, OrderRequest = OrderRequest, OrderId = OrderId, Order = Order>,
{
    fn clone(&self) -> Self {
        Self {
            request: self.request.clone(),
            venue: Arc::clone(&self.venue),
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
        )
    }
}

/// Transfer verb: move balance between two venues on (possibly
/// different) ledgers. Generic over both endpoints so a strategy
/// like "rebalance USDC from DriftV2 to Hyperliquid when ratio
/// > 0.7" types end-to-end.
pub struct Transfer<F: Ledger, T: Ledger> {
    pub amount_qty: nuke::domain::Notional,
    pub from: Arc<dyn LedgerHandle<F>>,
    pub to: Arc<dyn LedgerHandle<T>>,
}

impl<F: Ledger, T: Ledger> Clone for Transfer<F, T> {
    fn clone(&self) -> Self {
        Self {
            amount_qty: self.amount_qty,
            from: Arc::clone(&self.from),
            to: Arc::clone(&self.to),
        }
    }
}

impl<F: Ledger, T: Ledger> std::fmt::Debug for Transfer<F, T> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Transfer")
            .field("amount_qty", &self.amount_qty)
            .field("from", &self.from.name())
            .field("to", &self.to.name())
            .finish()
    }
}

/// Trait-object hook so [`Transfer`] can hold heterogeneous
/// "ledger handle" references without forcing a concrete `V`.
pub trait LedgerHandle<L: Ledger>: std::fmt::Debug + Send + Sync + 'static {
    fn name(&self) -> &'static str;
}

impl<F: Ledger, T: Ledger> Action for Transfer<F, T> {
    const KIND: &'static str = "verb.transfer";
    type Input = DecisionTag;
    type Output = TransferResult;

    fn lower<B, Err>(
        &self,
        dag: &DagFlow<B>,
        gate: &nuke::policy::PolicyGate<'_, B>,
    ) -> NodeHandle<Self::Input, Self::Output>
    where
        B: nuke::policy::LowerBackend<Self::Input, Self::Output, Err>,
        Err: Into<apalis_core::error::BoxDynError> + Send + 'static,
    {
        let from_name = self.from.name();
        let to_name = self.to.name();
        let amount_qty = self.amount_qty;
        let entry = nuke::policy::action::add_node(
            dag,
            "verb.transfer/initiate",
            move |verdict| async move {
                match verdict {
                    DecisionTag::Allow => TransferResult::Initiated {
                        from: from_name.to_string(),
                        to: to_name.to_string(),
                        amount_qty,
                    },
                    DecisionTag::Deny => TransferResult::Skipped {
                        verdict: DecisionTag::Deny,
                    },
                    DecisionTag::Escalate => TransferResult::Skipped {
                        verdict: DecisionTag::Escalate,
                    },
                }
            },
        );
        entry.depends_on(gate.builder())
    }
}

/// Outcome an order-submitting verb (Buy / Sell / Short) emits at
/// its terminal node. The real submit-and-wait flow will refine
/// `Submitted` into `Filled` / `Cancelled` / `Rejected` with the
/// venue's typed `OrderId`; the `Skipped` / `Failed` variants are
/// already useful so policy denials and venue errors stay in the
/// type instead of crashing the apalis worker.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum OrderResult {
    /// Order accepted by the venue. v0 records the `OrderId` returned
    /// by `TradingVenue::place_trade` plus the request `qty` for the
    /// downstream wait-for-fill node to key on.
    Submitted {
        id: nuke::OrderId,
        request_qty: nuke::domain::Qty,
    },
    /// Policy verdict was non-`Allow`; the verb skipped the venue
    /// call entirely. Carries the typed verdict tag so consumers can
    /// branch on `Deny` / `Escalate` without parsing strings.
    Skipped { verdict: DecisionTag },
    /// `TradingVenue::place_trade` returned an error.
    Failed { error: String },
}

/// Outcome a [`Transfer`] emits.
///
/// `Initiated` records the source/destination ledger names and the
/// notional moved. `Skipped` lands when the policy verdict gates the
/// transfer out (Deny / Escalate). Real LedgerHandle transfers will
/// add `Settled` / `Failed` variants once the trait grows a
/// transfer method.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum TransferResult {
    Initiated {
        from: String,
        to: String,
        amount_qty: nuke::domain::Notional,
    },
    Skipped {
        verdict: DecisionTag,
    },
}

/// `IntoResponse` lets a `task_fn` closure return one of these
/// directly. Apalis ships impls for primitives only; verb-specific
/// outcomes need their own. Both shapes are infallible from the
/// task's POV - failure is encoded in the variant.
impl apalis_core::task_fn::into_response::IntoResponse for OrderResult {
    type Output = Self;
    fn into_response(self) -> Result<Self, apalis_core::error::BoxDynError> {
        Ok(self)
    }
}

impl apalis_core::task_fn::into_response::IntoResponse for TransferResult {
    type Output = Self;
    fn into_response(self) -> Result<Self, apalis_core::error::BoxDynError> {
        Ok(self)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use evm::{EvmRpcVenue, EvmVenue, EvmVenueId};
    use nuke::OrderState;
    use nuke::domain::{Notional, Qty, Side, Symbol};
    use rust_decimal::Decimal;

    fn d(value: i64) -> Decimal {
        Decimal::from(value)
    }

    fn sample_request() -> OrderRequest {
        OrderRequest {
            instrument: Symbol::new("WETH/USDC"),
            side: Side::Buy,
            qty: Qty::new(d(1)),
            limit_px: None,
        }
    }

    fn sample_venue() -> Arc<EvmRpcVenue> {
        Arc::new(EvmRpcVenue {
            venue: EvmVenue {
                id: EvmVenueId {
                    address: alloy_primitives::address!("0000000000000000000000000000000000000001"),
                },
            },
        })
    }

    /// Concrete [`LedgerHandle`] used by the Transfer test below.
    /// Real strategies wire their own per-venue handles here.
    #[derive(Debug)]
    struct NamedLedger {
        name: &'static str,
    }

    impl<L: Ledger> LedgerHandle<L> for NamedLedger {
        fn name(&self) -> &'static str {
            self.name
        }
    }

    #[test]
    fn buy_carries_request_and_venue_and_kind() {
        let buy: Buy<EvmRpcVenue> = Buy {
            request: sample_request(),
            venue: sample_venue(),
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

    #[test]
    fn sell_kind_is_distinct_from_buy() {
        let sell: Sell<EvmRpcVenue> = Sell {
            request: sample_request(),
            venue: sample_venue(),
        };
        assert_eq!(<Sell<EvmRpcVenue> as Action>::KIND, "verb.sell");
        assert_eq!(sell.request.instrument, Symbol::new("WETH/USDC"));
    }

    #[test]
    fn short_kind_is_distinct_from_buy_and_sell() {
        let short: Short<EvmRpcVenue> = Short {
            request: sample_request(),
            venue: sample_venue(),
        };
        assert_eq!(<Short<EvmRpcVenue> as Action>::KIND, "verb.short");
        assert_eq!(short.request.qty, Qty::new(d(1)));
    }

    #[test]
    fn transfer_carries_amount_and_typed_endpoint_handles() {
        let transfer = Transfer::<EvmChain, EvmChain> {
            amount_qty: Notional::new(d(1_000)),
            from: Arc::new(NamedLedger { name: "drift-v2" }),
            to: Arc::new(NamedLedger {
                name: "hyperliquid",
            }),
        };
        assert_eq!(
            <Transfer<EvmChain, EvmChain> as Action>::KIND,
            "verb.transfer",
        );
        assert_eq!(transfer.from.name(), "drift-v2");
        assert_eq!(transfer.to.name(), "hyperliquid");
    }

    #[test]
    fn order_result_submitted_carries_request_qty_and_id() {
        let result = OrderResult::Submitted {
            id: nuke::OrderId::new("VENUE-1"),
            request_qty: Qty::new(d(7)),
        };
        match result {
            OrderResult::Submitted { request_qty, id } => {
                assert_eq!(request_qty, Qty::new(d(7)));
                assert_eq!(id, nuke::OrderId::new("VENUE-1"));
            }
            other => panic!("expected Submitted, got {other:?}"),
        }
    }

    #[test]
    fn order_result_skipped_carries_verdict_tag() {
        let result = OrderResult::Skipped {
            verdict: DecisionTag::Deny,
        };
        match result {
            OrderResult::Skipped { verdict } => assert_eq!(verdict, DecisionTag::Deny),
            other => panic!("expected Skipped, got {other:?}"),
        }
    }

    #[test]
    fn transfer_result_initiated_carries_endpoint_names() {
        let result = TransferResult::Initiated {
            from: "drift-v2".to_string(),
            to: "hyperliquid".to_string(),
            amount_qty: Notional::new(d(42)),
        };
        match result {
            TransferResult::Initiated { from, to, .. } => {
                assert_eq!(from, "drift-v2");
                assert_eq!(to, "hyperliquid");
            }
            other => panic!("expected Initiated, got {other:?}"),
        }
    }

    /// Mock [`TradingVenue<EvmChain>`] whose [`place_trade`] returns a
    /// preconfigured `Result`. Behaviour tests for [`submit`] use this
    /// to exercise the Allow / Deny / Escalate branches and the
    /// place_trade Ok / Err mapping without touching real RPC.
    #[derive(Debug)]
    struct MockVenue {
        venue: EvmVenue,
        outcome: MockOutcome,
    }

    #[derive(Debug, Clone)]
    enum MockOutcome {
        Ok(OrderId),
        Err(&'static str),
    }

    #[derive(Debug)]
    struct MockError(&'static str);

    impl std::fmt::Display for MockError {
        fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            write!(f, "{}", self.0)
        }
    }

    impl std::error::Error for MockError {}

    impl nuke::Venue<EvmChain> for MockVenue {
        type Id = EvmVenueId;
        type OrderId = OrderId;
        type OrderRequest = OrderRequest;
        type Order = Order;
        type Inventory = nuke::Inventory;

        fn id(&self) -> &Self::Id {
            &self.venue.id
        }
    }

    #[async_trait::async_trait]
    impl TradingVenue<EvmChain> for MockVenue {
        type Error = MockError;

        async fn check_inventory(&self) -> Result<Self::Inventory, Self::Error> {
            Err(MockError("not used in tests"))
        }

        async fn place_trade(
            &self,
            _request: Self::OrderRequest,
        ) -> Result<Self::OrderId, Self::Error> {
            match &self.outcome {
                MockOutcome::Ok(id) => Ok(id.clone()),
                MockOutcome::Err(msg) => Err(MockError(msg)),
            }
        }

        async fn check_order(&self, _id: &Self::OrderId) -> Result<Self::Order, Self::Error> {
            Err(MockError("not used in tests"))
        }
    }

    fn mock_venue(outcome: MockOutcome) -> Arc<MockVenue> {
        Arc::new(MockVenue {
            venue: EvmVenue {
                id: EvmVenueId {
                    address: alloy_primitives::address!("0000000000000000000000000000000000000002"),
                },
            },
            outcome,
        })
    }

    #[tokio::test]
    async fn submit_allow_with_ok_returns_submitted_with_passed_qty_and_id() {
        let venue = mock_venue(MockOutcome::Ok(OrderId::new("VENUE-OK")));
        let request = OrderRequest {
            qty: Qty::new(d(13)),
            ..sample_request()
        };
        let result = submit(DecisionTag::Allow, venue, request).await;
        assert_eq!(
            result,
            OrderResult::Submitted {
                id: OrderId::new("VENUE-OK"),
                request_qty: Qty::new(d(13)),
            }
        );
    }

    #[tokio::test]
    async fn submit_allow_with_err_returns_failed_carrying_formatted_error_string() {
        let venue = mock_venue(MockOutcome::Err("rpc unreachable"));
        let result = submit(DecisionTag::Allow, venue, sample_request()).await;
        // submit uses `format!("{e}")` (Display), not Debug, so the
        // payload should be the venue error's Display string verbatim.
        assert_eq!(
            result,
            OrderResult::Failed {
                error: "rpc unreachable".to_string(),
            }
        );
    }

    #[tokio::test]
    async fn submit_deny_skips_venue_and_returns_typed_skipped() {
        let venue = mock_venue(MockOutcome::Err("must not be called"));
        let result = submit(DecisionTag::Deny, venue, sample_request()).await;
        assert_eq!(
            result,
            OrderResult::Skipped {
                verdict: DecisionTag::Deny,
            }
        );
    }

    #[tokio::test]
    async fn submit_escalate_skips_venue_and_returns_typed_skipped() {
        let venue = mock_venue(MockOutcome::Err("must not be called"));
        let result = submit(DecisionTag::Escalate, venue, sample_request()).await;
        assert_eq!(
            result,
            OrderResult::Skipped {
                verdict: DecisionTag::Escalate,
            }
        );
    }
}
