//! `Transfer` verb: move balance between two venues on (possibly
//! different) ledgers, plus the [`LedgerHandle`] trait-object hook
//! and the [`TransferResult`] outcome it emits.

use std::sync::Arc;

use apalis_workflow::DagFlow;
use apalis_workflow::dag::NodeHandle;
use nuke::Ledger;
use nuke::policy::{Action, DecisionTag};
use serde::{Deserialize, Serialize};

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

/// Outcome a [`Transfer`] emits.
///
/// `Initiated` records the source/destination ledger names and the
/// notional moved. `Skipped` lands when the policy verdict gates the
/// transfer out (Deny / Escalate). Real LedgerHandle transfers will
/// add `Settled` / `Failed` variants once the trait grows a transfer
/// method.
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

impl apalis_core::task_fn::into_response::IntoResponse for TransferResult {
    type Output = Self;
    fn into_response(self) -> Result<Self, apalis_core::error::BoxDynError> {
        Ok(self)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::verbs::test_support::d;

    use evm::EvmChain;
    use nuke::domain::Notional;

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
}
