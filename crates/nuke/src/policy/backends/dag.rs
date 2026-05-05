//! Compile a [`RuleNode`] into an `apalis_workflow::DagFlow`.
//!
//! v2: the walker emits
//!
//! - one apalis task node per predicate (the `condition: InnerExpr`
//!   on every [`RuleNode::RejectIf`] / [`RuleNode::EscalateIf`] /
//!   [`RuleNode::Given`]). Signature: `fn(PolicyCtx) -> bool`.
//! - one apalis task node per compiled rule that runs the full
//!   [`crate::policy::evaluate`] pass against the same `PolicyCtx`
//!   and emits a [`DecisionTag`]. This is the verdict gate
//!   downstream gating / routing nodes will consume in the next
//!   iteration.
//! - the existing action sub-DAG nodes from [`Action::lower`].
//!
//! Combinators ([`RuleNode::All`] / [`RuleNode::Any`] /
//! [`RuleNode::Bind`] / [`RuleNode::Given`] gating) are still folded
//! into the verdict task body - emitting per-rule combinator nodes
//! is a future iteration. Actions are not yet gated by the verdict;
//! wiring `Action::lower` to depend on the verdict's `Allow` branch
//! is the next iteration after that.
//!
//! Eval errors inside a predicate node (missing field, type mismatch)
//! collapse to `false`. The verdict task surfaces the same errors as
//! `DecisionTag::Allow` - the runtime evaluator path remains the
//! source of truth for richer error reporting.

use apalis_core::backend::{BackendExt, codec::Codec};
use apalis_core::error::BoxDynError;
use apalis_core::task_fn::task_fn;
use apalis_workflow::DagFlow;

use crate::policy::action::Action;
use crate::policy::ast::{InnerExpr, RuleNode};
use crate::policy::ctx::PolicyCtx;
use crate::policy::decision::DecisionTag;
use crate::policy::eval::{eval_bool, evaluate};
use crate::policy::reason::Bindings;

/// Compile a [`RuleNode<A>`] into a fresh
/// [`apalis_workflow::DagFlow<B>`].
///
/// One predicate node per `RejectIf`/`EscalateIf`/`Given` condition,
/// plus one [`Action::lower`] sub-DAG per `Do` leaf. `name` is the
/// dag's display name (used by apalis-workflow's dot export and run
/// logging).
pub fn compile<B, A, Err>(rule: &RuleNode<A>, name: &str) -> DagFlow<B>
where
    B: BackendExt + Send + Sync + 'static,
    B::Context: Send + Sync + 'static,
    B::IdType: Send + Sync + 'static,
    A: Action + 'static,
    B::Codec: Codec<PolicyCtx, Compact = B::Compact, Error = Err>
        + Codec<bool, Compact = B::Compact, Error = Err>
        + Codec<DecisionTag, Compact = B::Compact, Error = Err>,
    Err: Into<BoxDynError> + Send + 'static,
{
    let dag = DagFlow::new(name);
    let mut idx = 0usize;
    walk(rule, &dag, &mut idx);
    emit_verdict(&dag, rule.clone());
    dag
}

fn walk<B, A, Err>(rule: &RuleNode<A>, dag: &DagFlow<B>, idx: &mut usize)
where
    B: BackendExt + Send + Sync + 'static,
    B::Context: Send + Sync + 'static,
    B::IdType: Send + Sync + 'static,
    A: Action,
    B::Codec: Codec<PolicyCtx, Compact = B::Compact, Error = Err>
        + Codec<bool, Compact = B::Compact, Error = Err>,
    Err: Into<BoxDynError> + Send + 'static,
{
    match rule {
        RuleNode::Do(action) => {
            // Lower the verb's sub-DAG. Discard the typed terminal
            // handle; v1 doesn't yet wire it into a verdict gate.
            let _terminal = action.lower(dag);
        }
        RuleNode::RejectIf { condition, .. } | RuleNode::EscalateIf { condition, .. } => {
            emit_predicate(dag, condition.clone(), *idx);
            *idx += 1;
        }
        RuleNode::Given { conditions, then } => {
            for condition in conditions {
                emit_predicate(dag, condition.clone(), *idx);
                *idx += 1;
            }
            walk(then, dag, idx);
        }
        RuleNode::Bind { then, .. } => walk(then, dag, idx),
        RuleNode::All(branches) | RuleNode::Any(branches) => {
            for sub in branches {
                walk(sub, dag, idx);
            }
        }
    }
}

fn emit_predicate<B, Err>(dag: &DagFlow<B>, predicate: InnerExpr, idx: usize)
where
    B: BackendExt + Send + Sync + 'static,
    B::Context: Send + Sync + 'static,
    B::IdType: Send + Sync + 'static,
    B::Codec: Codec<PolicyCtx, Compact = B::Compact, Error = Err>
        + Codec<bool, Compact = B::Compact, Error = Err>,
    Err: Into<BoxDynError> + Send + 'static,
{
    let name = format!("policy/predicate/{idx}");
    // Cloned once into the closure; each invocation re-borrows it.
    let _ = dag.add_node(
        &name,
        task_fn(move |ctx: PolicyCtx| {
            let predicate = predicate.clone();
            async move { eval_bool(&predicate, &ctx, &Bindings::empty()).unwrap_or(false) }
        }),
    );
}

fn emit_verdict<B, A, Err>(dag: &DagFlow<B>, rule: RuleNode<A>)
where
    B: BackendExt + Send + Sync + 'static,
    B::Context: Send + Sync + 'static,
    B::IdType: Send + Sync + 'static,
    A: Send + Sync + 'static,
    RuleNode<A>: Clone,
    B::Codec: Codec<PolicyCtx, Compact = B::Compact, Error = Err>
        + Codec<DecisionTag, Compact = B::Compact, Error = Err>,
    Err: Into<BoxDynError> + Send + 'static,
{
    let _ = dag.add_node(
        "policy/verdict",
        task_fn(move |ctx: PolicyCtx| {
            let rule = rule.clone();
            async move {
                evaluate(&rule, &ctx)
                    .map(|decision| decision.tag())
                    .unwrap_or(DecisionTag::Allow)
            }
        }),
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use apalis_file_storage::JsonStorage;
    use rust_decimal::Decimal;
    use serde_json::Value;

    use crate::domain::Qty;
    use crate::policy::ast::{Expr, QtyT, field, lt};
    use crate::policy::decision::RuleId;
    use crate::policy::reason::Reason;

    fn d(value: i64) -> Decimal {
        Decimal::from(value)
    }

    #[test]
    fn compiles_reject_if_to_one_predicate_node() {
        // One RejectIf -> one predicate node in the DAG.
        let rule: RuleNode = RuleNode::RejectIf {
            rule: RuleId::new("test.r"),
            condition: lt(
                field::<QtyT>("order", "qty"),
                Expr::<QtyT>::lit(Qty::new(d(10))),
            )
            .into_inner(),
            reason: Reason::literal("too small"),
        };

        let dag: DagFlow<JsonStorage<Value>> = compile(&rule, "policy");
        dag.validate().expect("dag has no cycles");

        // The dot export contains one node per emitted predicate.
        let dot = dag.to_dot();
        assert!(
            dot.contains("policy/predicate/0"),
            "expected predicate/0 in dag dot output:\n{dot}"
        );
        assert!(
            !dot.contains("policy/predicate/1"),
            "only one predicate expected:\n{dot}"
        );
    }

    #[test]
    fn compiles_all_combinator_to_one_predicate_node_per_branch() {
        // All [RejectIf, EscalateIf] -> two predicate nodes.
        let reject: RuleNode = RuleNode::RejectIf {
            rule: RuleId::new("test.a"),
            condition: lt(
                field::<QtyT>("order", "qty"),
                Expr::<QtyT>::lit(Qty::new(d(1))),
            )
            .into_inner(),
            reason: Reason::literal("too small"),
        };
        let escalate: RuleNode = RuleNode::EscalateIf {
            rule: RuleId::new("test.b"),
            condition: lt(
                field::<QtyT>("order", "qty"),
                Expr::<QtyT>::lit(Qty::new(d(2))),
            )
            .into_inner(),
            to: crate::policy::EscalationTarget::new("desk"),
            reason: Reason::literal("escalate"),
        };
        let rule: RuleNode = RuleNode::All(vec![reject, escalate]);

        let dag: DagFlow<JsonStorage<Value>> = compile(&rule, "policy");
        dag.validate().expect("dag has no cycles");

        let dot = dag.to_dot();
        assert!(
            dot.contains("policy/predicate/0") && dot.contains("policy/predicate/1"),
            "expected two predicate nodes in dot output:\n{dot}"
        );
    }

    #[test]
    fn compiles_action_free_rule_with_no_conditions_to_just_a_verdict_node() {
        // No RejectIf/EscalateIf/Given/Do -> walker emits no
        // predicates, but the verdict node still appears (so any
        // adopter who wants to gate on the verdict has a node to
        // depend on).
        let rule: RuleNode = RuleNode::All(vec![]);

        let dag: DagFlow<JsonStorage<Value>> = compile(&rule, "policy");
        dag.validate().expect("empty dag validates");
        let dot = dag.to_dot();
        assert!(!dot.contains("policy/predicate/"));
        assert!(
            dot.contains("policy/verdict"),
            "verdict node should always be emitted:\n{dot}"
        );
    }

    #[test]
    fn always_emits_a_verdict_node_alongside_predicates() {
        let rule: RuleNode = RuleNode::RejectIf {
            rule: RuleId::new("test.r"),
            condition: lt(
                field::<QtyT>("order", "qty"),
                Expr::<QtyT>::lit(Qty::new(d(10))),
            )
            .into_inner(),
            reason: Reason::literal("too small"),
        };

        let dag: DagFlow<JsonStorage<Value>> = compile(&rule, "policy");
        let dot = dag.to_dot();
        assert!(
            dot.contains("policy/predicate/0"),
            "predicate present:\n{dot}"
        );
        assert!(dot.contains("policy/verdict"), "verdict present:\n{dot}");
    }
}
