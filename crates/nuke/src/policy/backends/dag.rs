//! Compile a [`RuleNode`] into an `apalis_workflow::DagFlow`.
//!
//! v1: the walker emits one apalis task node per predicate (the
//! `condition: InnerExpr` carried by every [`RuleNode::RejectIf`] /
//! [`RuleNode::EscalateIf`] / [`RuleNode::Given`]) plus the existing
//! action sub-DAG nodes from [`Action::lower`]. The combinator
//! structure ([`RuleNode::All`] / [`RuleNode::Any`] /
//! [`RuleNode::Bind`] / [`RuleNode::Given`] gating) stays folded into
//! the runtime evaluator path - per-rule combinator nodes plus the
//! Allow-gate route to actions are the next iteration on this
//! backend.
//!
//! Each predicate node has signature `fn(PolicyCtx) -> bool`. The
//! `PolicyCtx` snapshot lives in [`crate::policy::ctx`] and travels
//! through the worker's chosen codec (typically JSON via
//! `apalis_file_storage::JsonStorage`). Eval errors (missing field,
//! type mismatch) collapse to `false` for v0; richer error
//! propagation lands when the verdict combinator node consumes
//! predicate outputs.

use apalis_core::backend::{BackendExt, codec::Codec};
use apalis_core::error::BoxDynError;
use apalis_core::task_fn::task_fn;
use apalis_workflow::DagFlow;

use crate::policy::action::Action;
use crate::policy::ast::{InnerExpr, RuleNode};
use crate::policy::ctx::PolicyCtx;
use crate::policy::eval::eval_bool;
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
    A: Action,
    B::Codec: Codec<PolicyCtx, Compact = B::Compact, Error = Err>
        + Codec<bool, Compact = B::Compact, Error = Err>,
    Err: Into<BoxDynError> + Send + 'static,
{
    let dag = DagFlow::new(name);
    let mut idx = 0usize;
    walk(rule, &dag, &mut idx);
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
    fn compiles_action_free_rule_with_no_conditions_to_an_empty_dagflow() {
        // No RejectIf/EscalateIf/Given/Do -> walker visits nothing.
        let rule: RuleNode = RuleNode::All(vec![]);

        let dag: DagFlow<JsonStorage<Value>> = compile(&rule, "policy");
        dag.validate().expect("empty dag validates");
        assert!(!dag.to_dot().contains("policy/predicate/"));
    }
}
