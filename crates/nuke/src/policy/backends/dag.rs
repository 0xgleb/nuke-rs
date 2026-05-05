//! Compile a [`RuleNode`] into an `apalis_workflow::DagFlow`.
//!
//! v0: the walker visits every [`RuleNode::Do`] leaf and calls
//! [`Action::lower`](crate::policy::action::Action::lower) so each
//! verb's sub-DAG is added to the returned DagFlow. Predicate /
//! guard / bind / combinator nodes are not yet emitted as their own
//! apalis tasks; the verdict path still runs through
//! [`crate::policy::evaluate`] for now. Per-node decomposition of the
//! verdict layer is the next iteration on this backend.

use apalis_core::backend::BackendExt;
use apalis_workflow::DagFlow;

use crate::policy::action::Action;
use crate::policy::ast::RuleNode;

/// Compile a [`RuleNode<A>`] into a fresh `apalis_workflow::DagFlow<B>`.
///
/// Each [`RuleNode::Do`] leaf in the tree results in one call to
/// [`Action::lower`], splicing the verb's sub-DAG into the returned
/// flow. The outer rule structure (predicates / guards / binds / All
/// / Any) is walked but does not yet emit nodes of its own; that's
/// the next iteration.
///
/// `name` is the dag's display name (used by apalis-workflow's dot
/// export and run logging).
pub fn compile<B, A>(rule: &RuleNode<A>, name: &str) -> DagFlow<B>
where
    B: BackendExt,
    A: Action,
{
    let dag = DagFlow::new(name);
    walk_actions(rule, &dag);
    dag
}

fn walk_actions<B, A>(rule: &RuleNode<A>, dag: &DagFlow<B>)
where
    B: BackendExt,
    A: Action,
{
    match rule {
        RuleNode::Do(action) => {
            // Discard the typed terminal handle; v0 doesn't yet
            // wire it as a dependency of any policy-side node.
            // Future: depend the action's entry on the upstream
            // guard's success branch.
            let _terminal = action.lower(dag);
        }
        RuleNode::Given { then, .. } | RuleNode::Bind { then, .. } => walk_actions(then, dag),
        RuleNode::All(branches) | RuleNode::Any(branches) => {
            for sub in branches {
                walk_actions(sub, dag);
            }
        }
        RuleNode::RejectIf { .. } | RuleNode::EscalateIf { .. } => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use apalis_core::backend::dequeue::VecDequeBackend;
    use rust_decimal::Decimal;

    use crate::domain::Qty;
    use crate::policy::ast::{Expr, QtyT, field, lt};
    use crate::policy::decision::RuleId;
    use crate::policy::reason::Reason;

    fn d(value: i64) -> Decimal {
        Decimal::from(value)
    }

    #[test]
    fn compiles_action_free_rule_to_an_empty_dagflow() {
        // No `Do` leaves -> the walker visits nothing actionable;
        // compile() still returns a real DagFlow that adopters can
        // hand to a WorkerBuilder. Verdict-side decomposition lands
        // in a follow-up.
        let rule: RuleNode = RuleNode::RejectIf {
            rule: RuleId::new("test.r"),
            condition: lt(
                field::<QtyT>("order", "qty"),
                Expr::<QtyT>::lit(Qty::new(d(10))),
            )
            .into_inner(),
            reason: Reason::literal("too small"),
        };

        let dag: DagFlow<VecDequeBackend<()>> = compile(&rule, "policy");
        // An empty dag has no cycles trivially.
        dag.validate().expect("empty dag validates");
    }
}
