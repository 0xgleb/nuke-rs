//! Concrete [`Action`] verbs the policy compiler lowers into apalis
//! sub-DAGs. The framework only ships the [`Action`] trait; verbs
//! are adopter code, demonstrated here for the cross-DEX arb
//! strategy.
//!
//! Each Buy / Sell / Short verb lowers to a single-node sub-DAG
//! whose closure runs the shared `submit` machinery: it gates on the
//! verdict and, on `Allow`, calls [`TradingVenue::place_trade`] on
//! the injected `Arc<V>`, mapping the result into [`OrderResult`]
//! (`Submitted` / `Failed`) or skipping with a typed `DecisionTag`
//! on `Deny` / `Escalate`. [`Transfer`] follows the same shape with
//! [`TransferResult`]. The verb's first node depends on the
//! `PolicyGate`'s verdict task, so the venue call only fires after
//! the per-rule verdict is computed.
//!
//! Layout:
//! - [`submit`]   - shared `lower_submit` / `submit` machinery and
//!   the [`OrderResult`] type the order verbs emit.
//! - [`buy`]      - [`Buy`] verb.
//! - [`sell`]     - [`Sell`] verb.
//! - [`short`]    - [`Short`] verb.
//! - [`transfer`] - [`Transfer`] verb plus [`LedgerHandle`] and
//!   [`TransferResult`].
//!
//! [`Action`]: nuke::policy::Action
//! [`TradingVenue::place_trade`]: nuke::TradingVenue::place_trade

mod buy;
mod sell;
mod short;
mod submit;
mod transfer;

pub use buy::Buy;
pub use sell::Sell;
pub use short::Short;
pub use submit::OrderResult;
pub use transfer::Transfer;

#[cfg(test)]
mod test_support {
    use std::sync::Arc;
    use std::time::Duration;

    use evm::{EvmRpcVenue, EvmVenue, EvmVenueId};
    use nuke::OrderRequest;
    use nuke::domain::{Qty, Side, Symbol};
    use rust_decimal::Decimal;

    pub(super) fn d(value: i64) -> Decimal {
        Decimal::from(value)
    }

    pub(super) fn sample_request() -> OrderRequest {
        OrderRequest {
            instrument: Symbol::new("WETH/USDC"),
            side: Side::Buy,
            qty: Qty::new(d(1)),
            limit_px: None,
        }
    }

    pub(super) fn sample_venue() -> Arc<EvmRpcVenue> {
        Arc::new(EvmRpcVenue {
            venue: EvmVenue {
                id: EvmVenueId {
                    address: alloy_primitives::address!("0000000000000000000000000000000000000001"),
                },
            },
        })
    }

    /// Generous timeout for tests that don't exercise the elapsed
    /// path - large enough that the mock's synchronous Ok / Err
    /// completes well before it fires.
    pub(super) fn sample_timeout() -> Duration {
        Duration::from_secs(60)
    }
}
