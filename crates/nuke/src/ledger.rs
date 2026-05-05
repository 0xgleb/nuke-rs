//! [`Ledger`] - the settlement substrate abstraction.
//!
//! A `Ledger` is "where transactions get settled and what an order /
//! account / balance looks like there". It is the per-substrate
//! anchor for the value objects every Venue on that substrate
//! shares.
//!
//! Concrete examples (defined in adapter crates, not here):
//!
//! - EVM chains: a `crates/evm` adapter defines `EvmChain { chain_id }`
//!   plus pre-defined constants for mainnet / Arbitrum / Base / etc.
//! - SVM chains: a `crates/svm` adapter defines `SvmCluster`.
//! - Centralized exchanges: each adapter defines its own ledger marker
//!   (e.g. `BinanceCex`, `CoinbaseCex`).
//! - Paper-trading sandboxes for back-testing.
//!
//! The trait is intentionally minimal. Add to it only what *every*
//! `Ledger` impl needs - over-abstracting locks adopters out.

use std::fmt::{Debug, Display};

/// The settlement substrate a [`Venue`](crate::Venue) lives on.
///
/// Implementations are zero-sized markers (or thin newtypes around a
/// chain id / cluster name) defined in adapter crates. Code generic
/// over `L: Ledger` can address venue value objects through `L`'s
/// associated types without knowing the substrate.
pub trait Ledger: Send + Sync + 'static {
    /// Stable identifier used in logs / metrics / audit trails.
    /// e.g. `"ethereum-mainnet"`, `"solana-mainnet"`, `"binance-spot"`.
    const NAME: &'static str;

    /// Address shape on this ledger. EVM: 20-byte address newtype;
    /// SVM: pubkey newtype; CEX: account-id newtype.
    type Address: Debug + Display + Clone + Send + Sync + 'static;

    /// Identifier shape for a settled transaction (or its CEX
    /// equivalent). Returned by `place_trade` so the caller can later
    /// `check_order` against the ledger.
    type TxId: Debug + Display + Clone + Send + Sync + 'static;
}
