//! Shared submit machinery for the [`Buy`](super::Buy) /
//! [`Sell`](super::Sell) / [`Short`](super::Short) verbs, plus the
//! [`OrderResult`] type they emit at their terminal node.
//!
//! The body of [`submit`] is identical across the three verbs, so it
//! lives here and is invoked from each verb's `Action::lower` via
//! [`lower_submit`].

use std::sync::Arc;
use std::time::Duration;

use apalis_workflow::DagFlow;
use apalis_workflow::dag::NodeHandle;
use evm::EvmChain;
use nuke::policy::DecisionTag;
use nuke::{Order, OrderId, OrderRequest, TradingVenue};
use serde::{Deserialize, Serialize};

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

/// `IntoResponse` lets a `task_fn` closure return one of these
/// directly. Apalis ships impls for primitives only; verb-specific
/// outcomes need their own. The shape is infallible from the task's
/// POV - failure is encoded in the variant.
impl apalis_core::task_fn::into_response::IntoResponse for OrderResult {
    type Output = Self;
    fn into_response(self) -> Result<Self, apalis_core::error::BoxDynError> {
        Ok(self)
    }
}

/// Shared lowering for [`Buy`](super::Buy) / [`Sell`](super::Sell) /
/// [`Short`](super::Short): add a single submit node under the given
/// name whose closure clones the captured `venue` / `request` per
/// invocation, runs [`submit`], and depends on the verdict gate.
/// Each verb wires its own `KIND`-keyed node name (e.g.
/// `verb.buy/submit`) so the DAG dot output stays readable, but the
/// body is identical so it lives here.
pub(super) fn lower_submit<V, B, Err>(
    dag: &DagFlow<B>,
    gate: &nuke::policy::PolicyGate<'_, B>,
    node_name: &str,
    venue: Arc<V>,
    request: OrderRequest,
    place_timeout: Duration,
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
        async move { submit(verdict, venue, request, place_timeout).await }
    });
    entry.depends_on(gate.builder())
}

/// Shared closure body for [`Buy`](super::Buy) /
/// [`Sell`](super::Sell) / [`Short`](super::Short): gate on the
/// verdict, otherwise call `venue.place_trade(request)` under a
/// `tokio::time::timeout(place_timeout, ...)` so a stalled venue can
/// never hang the reactor.
///
/// Failure shape on `Allow`:
/// - `Ok(id)`                                  -> `Submitted { id, request_qty }`
/// - `Err(error)`                              -> `Failed { error: format!("{error}") }`
/// - timeout (`tokio::time::error::Elapsed`)   -> `Failed { error: "timeout: ..." }`
///
/// The `timeout:` prefix lets callers classify transient venue stalls
/// without re-parsing arbitrary RPC error strings.
async fn submit<V>(
    verdict: DecisionTag,
    venue: Arc<V>,
    request: OrderRequest,
    place_timeout: Duration,
) -> OrderResult
where
    V: TradingVenue<EvmChain, OrderRequest = OrderRequest, OrderId = OrderId, Order = Order>,
{
    let request_qty = request.qty;
    match verdict {
        DecisionTag::Allow => {
            match tokio::time::timeout(place_timeout, venue.place_trade(request)).await {
                Ok(Ok(id)) => OrderResult::Submitted { id, request_qty },
                Ok(Err(error)) => OrderResult::Failed {
                    error: format!("{error}"),
                },
                Err(_elapsed) => OrderResult::Failed {
                    error: format!("timeout: place_trade exceeded {place_timeout:?}"),
                },
            }
        }
        DecisionTag::Deny => OrderResult::Skipped {
            verdict: DecisionTag::Deny,
        },
        DecisionTag::Escalate => OrderResult::Skipped {
            verdict: DecisionTag::Escalate,
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::verbs::test_support::{d, sample_request, sample_timeout};

    use evm::{EvmVenue, EvmVenueId};
    use nuke::domain::Qty;

    /// Mock [`TradingVenue<EvmChain>`] whose `place_trade` returns a
    /// preconfigured outcome. Behaviour tests for [`submit`] use this
    /// to exercise the Allow / Deny / Escalate branches and the
    /// place_trade Ok / Err / timeout mappings without touching real
    /// RPC.
    #[derive(Debug)]
    struct MockVenue {
        venue: EvmVenue,
        outcome: MockOutcome,
    }

    #[derive(Debug, Clone)]
    enum MockOutcome {
        Ok(OrderId),
        Err(&'static str),
        /// Sleep `Duration` before returning - used to drive the
        /// `tokio::time::timeout` elapsed branch in [`submit`].
        Slow(Duration),
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
                MockOutcome::Slow(duration) => {
                    tokio::time::sleep(*duration).await;
                    Err(MockError("slow path completed past timeout"))
                }
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

    #[tokio::test]
    async fn submit_allow_with_ok_returns_submitted_with_passed_qty_and_id() {
        let venue = mock_venue(MockOutcome::Ok(OrderId::new("VENUE-OK")));
        let request = OrderRequest {
            qty: Qty::new(d(13)),
            ..sample_request()
        };
        let result = submit(DecisionTag::Allow, venue, request, sample_timeout()).await;
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
        let result = submit(
            DecisionTag::Allow,
            venue,
            sample_request(),
            sample_timeout(),
        )
        .await;
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
        let result = submit(DecisionTag::Deny, venue, sample_request(), sample_timeout()).await;
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
        let result = submit(
            DecisionTag::Escalate,
            venue,
            sample_request(),
            sample_timeout(),
        )
        .await;
        assert_eq!(
            result,
            OrderResult::Skipped {
                verdict: DecisionTag::Escalate,
            }
        );
    }

    #[tokio::test]
    async fn submit_allow_classifies_elapsed_timeout_into_failed_with_timeout_prefix() {
        // Mock sleeps for 1s; submit's place_timeout is 10ms, so the
        // tokio::time::timeout fires before place_trade returns. The
        // outcome must be Failed with a `"timeout:"`-prefixed string
        // so callers can branch on stalls vs. RPC errors without
        // re-parsing arbitrary venue messages.
        let venue = mock_venue(MockOutcome::Slow(Duration::from_secs(1)));
        let place_timeout = Duration::from_millis(10);
        let result = submit(DecisionTag::Allow, venue, sample_request(), place_timeout).await;
        match result {
            OrderResult::Failed { error } => {
                assert!(
                    error.starts_with("timeout:"),
                    "expected timeout-prefixed error, got: {error}",
                );
            }
            other => panic!("expected Failed, got {other:?}"),
        }
    }
}
