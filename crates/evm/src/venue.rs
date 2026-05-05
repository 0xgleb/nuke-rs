//! [`EvmChain`] (Ledger), [`EvmVenue`] (Venue<EvmChain>), and a
//! placeholder [`EvmRpcVenue`] (TradingVenue<EvmChain>) that takes
//! `OrderRequest`s and submits them as signed transactions.
//!
//! v0 emits `place_trade` returning a stub error: the actual
//! signed-tx flow needs a signer + fee oracle + nonce manager, which
//! adopters wire when they're ready. The trait surface is in place
//! so [`crate::pump`] can run a reactor that *gates* opportunities
//! through the framework's `Validator` chain even before the trade
//! submission path is real.

use alloy_primitives::{Address, B256};
use async_trait::async_trait;
use nuke::{Inventory, Ledger, Order, OrderId, OrderRequest, TradingVenue, Venue};

/// Ledger marker for an EVM chain. The chain id distinguishes
/// mainnet (1) / Arbitrum (42161) / Base (8453) / etc.
#[derive(Debug, Clone, Copy)]
pub struct EvmChain {
    pub chain_id: u64,
}

impl Ledger for EvmChain {
    const NAME: &'static str = "evm";
    type Address = Address;
    /// Transaction hash on this chain.
    type TxId = B256;
}

/// A specific tradeable contract on an EVM chain. v0 keeps the venue
/// id as a contract address; richer adapters wrap (chain, factory,
/// pool) in their own type.
#[derive(Debug, Clone)]
pub struct EvmVenueId {
    pub address: Address,
}

impl std::fmt::Display for EvmVenueId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.address)
    }
}

/// Read-side EVM venue identification. The trading-side write
/// primitives live on [`EvmRpcVenue`] which extends this.
#[derive(Debug, Clone)]
pub struct EvmVenue {
    pub id: EvmVenueId,
}

impl Venue<EvmChain> for EvmVenue {
    type Id = EvmVenueId;
    type OrderId = OrderId;
    type OrderRequest = OrderRequest;
    type Order = Order;
    type Inventory = Inventory;

    fn id(&self) -> &Self::Id {
        &self.id
    }
}

/// EVM trading-venue stub: the trait surface is wired up; the
/// actual signed-tx submission is left for adopters that bring a
/// signer.
pub struct EvmRpcVenue {
    pub venue: EvmVenue,
}

impl Venue<EvmChain> for EvmRpcVenue {
    type Id = EvmVenueId;
    type OrderId = OrderId;
    type OrderRequest = OrderRequest;
    type Order = Order;
    type Inventory = Inventory;

    fn id(&self) -> &Self::Id {
        &self.venue.id
    }
}

#[derive(Debug, thiserror::Error)]
pub enum EvmVenueError {
    /// The trade-submission path (sign + send + wait-for-receipt)
    /// hasn't been wired yet for this venue. Adopters provide a
    /// signer and replace this stub with their concrete impl.
    #[error("EVM trade submission not wired for this venue")]
    Unwired,
}

#[async_trait]
impl TradingVenue<EvmChain> for EvmRpcVenue {
    type Error = EvmVenueError;

    async fn check_inventory(&self) -> Result<Self::Inventory, Self::Error> {
        Err(EvmVenueError::Unwired)
    }

    async fn place_trade(
        &self,
        _request: Self::OrderRequest,
    ) -> Result<Self::OrderId, Self::Error> {
        Err(EvmVenueError::Unwired)
    }

    async fn check_order(&self, _id: &Self::OrderId) -> Result<Self::Order, Self::Error> {
        Err(EvmVenueError::Unwired)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloy_primitives::address;

    #[test]
    fn evm_chain_implements_ledger_with_chain_id() {
        let _chain = EvmChain { chain_id: 1 };
        assert_eq!(<EvmChain as Ledger>::NAME, "evm");
    }

    #[test]
    fn evm_venue_id_is_displayable() {
        let id = EvmVenueId {
            address: address!("0000000000000000000000000000000000000001"),
        };
        assert!(id.to_string().starts_with("0x"));
    }

    #[tokio::test]
    async fn evm_rpc_venue_returns_unwired_for_v0_trade_path() {
        let venue = EvmRpcVenue {
            venue: EvmVenue {
                id: EvmVenueId {
                    address: address!("0000000000000000000000000000000000000001"),
                },
            },
        };
        let err = venue.check_inventory().await.expect_err("v0 stub");
        assert!(matches!(err, EvmVenueError::Unwired));
    }
}
