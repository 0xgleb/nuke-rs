//! [`Venue`] (any market on a [`Ledger`]) and its write-side
//! extension [`TradingVenue`] (the `check_inventory` /
//! `place_trade` / `check_order` primitives).
//!
//! Read-side venue feeds live in [`crate::feed`] - a separate trait
//! so credentials don't leak into pure observation paths and a
//! single crate can implement either side independently.
//!
//! Value objects (`OrderRequest`, `OrderId`, `Order`, `Inventory`)
//! are defined per-venue via associated types so each adapter chooses
//! its own shape. Common defaults will be added once patterns settle.

use std::fmt::{Debug, Display};

use async_trait::async_trait;

use crate::ledger::Ledger;

/// A specific tradeable market on a settlement substrate `L`.
///
/// `Venue` carries the identifying info typed against `L` (a
/// contract address on EVM; a market account on SVM; a `(symbol,
/// market_kind)` pair on a CEX). `TradingVenue` extends it with the
/// write-side primitives.
pub trait Venue<L: Ledger>: Send + Sync + 'static {
    /// Identifier for this venue on `L`. CEX market symbol; DEX pool
    /// address; etc.
    type Id: Debug + Display + Clone + Send + Sync + 'static;

    /// Order id shape returned when an order is accepted by the
    /// venue. Often (but not always) the same shape as `L::TxId`.
    type OrderId: Debug + Display + Clone + Send + Sync + 'static;

    /// Order request shape (what the user submits). Carries side,
    /// quantity, price, time-in-force, etc.
    type OrderRequest: Debug + Send + Sync + 'static;

    /// Order shape (what the venue returns on `check_order`).
    /// Carries the request, current state, fills, etc.
    type Order: Debug + Send + Sync + 'static;

    /// Inventory shape (balances / positions held at this venue).
    type Inventory: Debug + Send + Sync + 'static;

    /// This venue's id.
    fn id(&self) -> &Self::Id;
}

/// Write-side trait: the three core primitives every tradeable venue
/// supports.
///
/// Implementations almost always perform real I/O (signed-tx
/// submission / REST POST / etc.) and so are expected to be called
/// from inside an apalis Job (so retries + durability + backoff are
/// free). The framework's run loop never invokes a `TradingVenue`
/// method directly.
#[async_trait]
pub trait TradingVenue<L: Ledger>: Venue<L> {
    /// Errors this venue's API can return.
    type Error: std::error::Error + Send + Sync + 'static;

    /// Read current inventory (balances + positions) at this venue.
    async fn check_inventory(&self) -> Result<Self::Inventory, Self::Error>;

    /// Submit an order request. Returns the venue-assigned order id
    /// once accepted; further status (fills, cancels) requires a
    /// follow-up [`check_order`](Self::check_order) or a separate
    /// order-feed stream from [`crate::feed`].
    async fn place_trade(&self, request: Self::OrderRequest) -> Result<Self::OrderId, Self::Error>;

    /// Query the current state of a previously-placed order.
    async fn check_order(&self, id: &Self::OrderId) -> Result<Self::Order, Self::Error>;
}
