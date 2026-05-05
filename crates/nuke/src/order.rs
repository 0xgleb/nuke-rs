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

use std::fmt;

use serde::{Deserialize, Serialize};

use crate::Identifier;
use crate::domain::{Notional, Px, Qty, Side, Symbol};

/// A request to place an order, prior to acceptance by a venue. The
/// venue assigns the [`OrderId`] when it acks the request.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OrderRequest {
    pub instrument: Symbol,
    pub side: Side,
    pub qty: Qty,
    /// Limit price. `None` for market orders.
    pub limit_px: Option<Px>,
}

/// Stable typed identifier for an order accepted by a venue. The
/// inner string is whatever the venue returns; the typed wrapper
/// prevents mixing it up with other ids.
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
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Order {
    pub id: OrderId,
    pub request: OrderRequest,
    pub state: OrderState,
    /// Quantity filled so far. Always `<= request.qty`.
    pub filled: Qty,
}

impl Identifier<OrderId> for Order {
    fn id(&self) -> &OrderId {
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
/// One entry per (instrument) pair. Adopters typically rebuild this
/// from the venue's account stream and feed it into a
/// [`crate::Validator`] that gates new [`OrderRequest`]s.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Inventory {
    entries: Vec<InventoryEntry>,
}

/// One row in the [`Inventory`]: the instrument and its available
/// notional balance.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct InventoryEntry {
    pub instrument: Symbol,
    pub available: Notional,
}

impl Inventory {
    pub fn empty() -> Self {
        Self {
            entries: Vec::new(),
        }
    }

    pub fn from_entries<I>(entries: I) -> Self
    where
        I: IntoIterator<Item = InventoryEntry>,
    {
        Self {
            entries: entries.into_iter().collect(),
        }
    }

    /// Look up the available balance for `instrument`. Returns
    /// [`Notional`] zero (via `None`) if the inventory has no entry.
    pub fn available(&self, instrument: &Symbol) -> Option<&Notional> {
        self.entries
            .iter()
            .find(|entry| &entry.instrument == instrument)
            .map(|entry| &entry.available)
    }

    pub fn iter(&self) -> impl Iterator<Item = &InventoryEntry> {
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
}
