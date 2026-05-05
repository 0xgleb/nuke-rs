//! Concrete [`Action`] verbs the policy compiler lowers into apalis
//! sub-DAGs. The framework only ships the [`Action`] trait; verbs
//! are adopter code, demonstrated here for the cross-DEX arb
//! strategy.
//!
//! v0 lowers each verb to a single-node sub-DAG that captures the
//! verb's inputs and (in the real impl) calls
//! [`TradingVenue::place_trade`] / [`TradingVenue::check_inventory`]
//! / etc. on an `Arc<V>` injected via `nuke::Job` context. For the
//! example we keep the closure body minimal - the value here is the
//! Action trait surface, not the venue I/O. Wiring the actual
//! [`TradingVenue::place_trade`] flow lands when the per-node
//! verdict-layer decomposition (the policy walker that currently
//! only lowers `Do` leaves) is fleshed out.

use std::marker::PhantomData;
use std::sync::Arc;

use apalis_core::backend::BackendExt;
use apalis_workflow::DagFlow;
use apalis_workflow::dag::NodeHandle;
use evm::EvmChain;
use nuke::policy::Action;
use nuke::{Ledger, Order, OrderId, OrderRequest, TradingVenue};
use serde::{Deserialize, Serialize};

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
    type Input = OrderRequest;
    type Output = OrderResult;

    fn lower<B: BackendExt>(&self, _dag: &DagFlow<B>) -> NodeHandle<Self::Input, Self::Output> {
        // v0 surface only: the verb compiles, can be embedded in a
        // `policy! { do Buy { ... } }` construction, and shows up
        // in audit / mermaid renders by KIND. The real
        // submit-and-wait sub-DAG (one node calling
        // `self.venue.place_trade(req)`, a downstream wait-for-fill
        // node) lands when the per-node verdict-layer decomposition
        // is wired (the policy walker that currently only lowers
        // `Do` leaves). Stubbing `unreachable!` keeps the trait
        // surface honest without forcing the full apalis-workflow
        // Codec/DagCodec bound chain on every verb.
        unreachable!("Buy::lower v0 stub - wired in #80 follow-up")
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
    type Input = OrderRequest;
    type Output = OrderResult;

    fn lower<B: BackendExt>(&self, _dag: &DagFlow<B>) -> NodeHandle<Self::Input, Self::Output> {
        unreachable!("Sell::lower v0 stub - wired in #80 follow-up")
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
    type Input = OrderRequest;
    type Output = OrderResult;

    fn lower<B: BackendExt>(&self, _dag: &DagFlow<B>) -> NodeHandle<Self::Input, Self::Output> {
        unreachable!("Short::lower v0 stub - wired in #80 follow-up")
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
    type Input = TransferRequest;
    type Output = TransferResult;

    fn lower<B: BackendExt>(&self, _dag: &DagFlow<B>) -> NodeHandle<Self::Input, Self::Output> {
        unreachable!("Transfer::lower v0 stub - wired in #80 follow-up")
    }
}

/// Input the Transfer verb takes: amount + a phantom (the typed
/// ledgers sit on the `Transfer` struct itself, not in this Input).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TransferRequest {
    pub amount_qty: nuke::domain::Notional,
    #[serde(skip)]
    pub _phantom: PhantomData<()>,
}

/// Outcome an order-submitting verb (Buy / Sell / Short) emits at
/// its terminal node. v0 only models "submitted"; the real
/// submit-and-wait flow will swap this for a `Filled` / `Cancelled`
/// / `Rejected` discriminator with the venue's typed `OrderId`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum OrderResult {
    Submitted { request_qty: nuke::domain::Qty },
}

/// Outcome a [`Transfer`] emits.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum TransferResult {
    Initiated {
        from: &'static str,
        to: &'static str,
        amount_qty: nuke::domain::Notional,
    },
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
    fn transfer_request_round_trips_through_serde() {
        let original = TransferRequest {
            amount_qty: Notional::new(d(42)),
            _phantom: PhantomData,
        };
        let bytes = serde_json::to_vec(&original).unwrap();
        let parsed: TransferRequest = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(parsed, original);
    }

    #[test]
    fn order_result_submitted_carries_request_qty() {
        let result = OrderResult::Submitted {
            request_qty: Qty::new(d(7)),
        };
        match result {
            OrderResult::Submitted { request_qty } => {
                assert_eq!(request_qty, Qty::new(d(7)));
            }
        }
    }

    #[test]
    fn transfer_result_initiated_carries_endpoint_names() {
        let result = TransferResult::Initiated {
            from: "drift-v2",
            to: "hyperliquid",
            amount_qty: Notional::new(d(42)),
        };
        match result {
            TransferResult::Initiated { from, to, .. } => {
                assert_eq!(from, "drift-v2");
                assert_eq!(to, "hyperliquid");
            }
        }
    }
}
