//! Typed value objects for the order lifecycle: [`OrderRequest`],
//! [`OrderId`], [`Order`], [`OrderState`], plus [`Inventory`] for
//! pre-trade balance checks.
//!
//! These are the framework's *vocabulary*, not its
//! implementation. A `TradingVenue` maps adopter-side concepts
//! (Uniswap-V2 swap, Hyperliquid limit order, ...) onto these
//! shapes; a reactor's pre-trade [`crate::Validator`] reads
//! [`Inventory`] and the current [`OrderState`] to decide whether
//! to allow the next [`OrderRequest`].
//!
//! ## Generic-over-key-types
//!
//! [`OrderRequest`], [`Order`], [`Inventory`], and [`InventoryEntry`]
//! are generic over an instrument key `I` (default [`Symbol`]) and -
//! for [`Order`] - an order-id key `K` (default [`OrderId`]). Adopters
//! using indexed instruments (e.g. `u32`) can use
//! `OrderRequest<u32>` / `Inventory<u32>` without forking the value
//! objects. The defaults keep ergonomics for the common
//! `Symbol`-keyed case.

use std::fmt;

use serde::{Deserialize, Serialize};

use crate::Identifier;
use crate::domain::{Notional, Px, Qty, Side, Symbol};

/// A request to place an order, prior to acceptance by a venue. The
/// venue assigns the order id when it acks the request.
///
/// Generic over instrument key `I` (default [`Symbol`]).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OrderRequest<I = Symbol> {
    pub instrument: I,
    pub side: Side,
    pub qty: Qty,
    /// Limit price. `None` for market orders.
    pub limit_px: Option<Px>,
}

/// Default order-id key type used by [`Order`]. Adopters who need a
/// different shape (e.g. `u64`, a uuid) can substitute it via the
/// `K` parameter on [`Order`].
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub struct OrderId(String);

impl OrderId {
    pub fn new(value: impl Into<String>) -> Self {
        Self(value.into())
    }
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for OrderId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

/// An order tracked through its lifecycle: the original request, the
/// venue-assigned id, the current state, and the cumulative
/// fill-side bookkeeping.
///
/// Generic over instrument key `I` (default [`Symbol`]) and order-id
/// key `K` (default [`OrderId`]).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Order<I = Symbol, K = OrderId> {
    pub id: K,
    pub request: OrderRequest<I>,
    pub state: OrderState,
    /// Quantity filled so far. Always `<= request.qty`.
    pub filled: Qty,
}

impl<I, K> Identifier<K> for Order<I, K> {
    fn id(&self) -> &K {
        &self.id
    }
}

/// Lifecycle state of an [`Order`]. Adopters can implement their own
/// venue-specific state machines that map onto this for the
/// framework's bookkeeping.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum OrderState {
    /// Submitted to the venue, no fills yet.
    Open,
    /// At least one fill has landed but the order is not yet
    /// fully filled.
    PartiallyFilled,
    /// The order's full quantity has been filled.
    Filled,
    /// The venue (or the operator) cancelled before full fill.
    Cancelled,
    /// The venue rejected the order.
    Rejected,
}

impl OrderState {
    /// True iff the order is no longer expected to receive fills.
    pub fn is_terminal(&self) -> bool {
        matches!(self, Self::Filled | Self::Cancelled | Self::Rejected)
    }
}

/// Available balance the reactor's pre-trade check reads.
///
/// One entry per instrument. Adopters typically rebuild this from the
/// venue's account stream and feed it into a [`crate::Validator`]
/// that gates new [`OrderRequest`]s.
///
/// Generic over instrument key `I` (default [`Symbol`]).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Inventory<I = Symbol> {
    entries: Vec<InventoryEntry<I>>,
}

impl<I> Default for Inventory<I> {
    fn default() -> Self {
        Self {
            entries: Vec::new(),
        }
    }
}

/// One row in the [`Inventory`]: the instrument and its available
/// notional balance.
///
/// Generic over instrument key `I` (default [`Symbol`]).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct InventoryEntry<I = Symbol> {
    pub instrument: I,
    pub available: Notional,
}

impl<I> Inventory<I> {
    pub fn empty() -> Self {
        Self {
            entries: Vec::new(),
        }
    }

    pub fn from_entries<It>(entries: It) -> Self
    where
        It: IntoIterator<Item = InventoryEntry<I>>,
    {
        Self {
            entries: entries.into_iter().collect(),
        }
    }

    /// Look up the available balance for `instrument`. Returns `None`
    /// if the inventory has no entry for it.
    pub fn available(&self, instrument: &I) -> Option<&Notional>
    where
        I: PartialEq,
    {
        self.entries
            .iter()
            .find(|entry| &entry.instrument == instrument)
            .map(|entry| &entry.available)
    }

    pub fn iter(&self) -> impl Iterator<Item = &InventoryEntry<I>> {
        self.entries.iter()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rust_decimal::Decimal;

    fn d(value: i64) -> Decimal {
        Decimal::from(value)
    }

    #[test]
    fn order_state_terminal_includes_filled_cancelled_and_rejected_only() {
        assert!(OrderState::Filled.is_terminal());
        assert!(OrderState::Cancelled.is_terminal());
        assert!(OrderState::Rejected.is_terminal());
        assert!(!OrderState::Open.is_terminal());
        assert!(!OrderState::PartiallyFilled.is_terminal());
    }

    #[test]
    fn order_implements_identifier_for_its_id() {
        let id = OrderId::new("VENUE-42");
        let order = Order {
            id: id.clone(),
            request: OrderRequest {
                instrument: Symbol::new("WETH/USDC"),
                side: Side::Buy,
                qty: Qty::new(d(1)),
                limit_px: None,
            },
            state: OrderState::Open,
            filled: Qty::new(d(0)),
        };
        let borrowed: &OrderId = order.id();
        assert_eq!(*borrowed, id);
    }

    #[test]
    fn inventory_available_returns_some_for_known_and_none_for_unknown() {
        let symbol_a = Symbol::new("WETH/USDC");
        let symbol_b = Symbol::new("WBTC/USDC");
        let inventory = Inventory::from_entries([InventoryEntry {
            instrument: symbol_a.clone(),
            available: Notional::new(d(1_000)),
        }]);

        assert_eq!(
            inventory.available(&symbol_a),
            Some(&Notional::new(d(1_000)))
        );
        assert_eq!(inventory.available(&symbol_b), None);
    }

    #[test]
    fn inventory_can_be_keyed_by_a_non_default_instrument_type() {
        let inventory: Inventory<u32> = Inventory::from_entries([
            InventoryEntry {
                instrument: 1,
                available: Notional::new(d(100)),
            },
            InventoryEntry {
                instrument: 2,
                available: Notional::new(d(250)),
            },
        ]);

        assert_eq!(inventory.available(&1), Some(&Notional::new(d(100))));
        assert_eq!(inventory.available(&2), Some(&Notional::new(d(250))));
        assert_eq!(inventory.available(&3), None);
    }

    #[test]
    fn order_can_be_parameterized_by_custom_keys() {
        let order: Order<u32, u64> = Order {
            id: 7,
            request: OrderRequest {
                instrument: 1,
                side: Side::Sell,
                qty: Qty::new(d(2)),
                limit_px: None,
            },
            state: OrderState::Open,
            filled: Qty::new(d(0)),
        };
        let borrowed: &u64 = order.id();
        assert_eq!(*borrowed, 7);
    }
}
