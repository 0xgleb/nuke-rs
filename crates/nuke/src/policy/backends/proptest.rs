//! Proptest scaffolding backend — derives a structural test plan from
//! a `RuleNode`. Coverage is structural (one generated test per leaf
//! branch), not line-based.
//!
//! V0 emits a [`TestPlan`] describing the branches that need
//! exercising; the actual proptest strategies require domain-typed
//! generators which arrive once `derive(Domain)` propagates field
//! type info into this layer. Until then, the plan tells you *which*
//! branches you need property tests for.

use crate::policy::ast::{InnerExpr, RuleNode};
use crate::policy::decision::RuleId;

/// One leaf branch in the rule's decision tree.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LeafBranch {
    pub rule: RuleId,
    pub kind: BranchKind,
    /// Expression rendered for documentation / cross-reference.
    pub condition: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BranchKind {
    Reject,
    Escalate,
}

/// A complete plan: one [`LeafBranch`] per `RejectIf`/`EscalateIf`
/// reachable in the rule. Each branch needs at least one property
/// test asserting that the condition holds *and* the expected
/// decision is produced.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct TestPlan {
    pub branches: Vec<LeafBranch>,
}

/// Walk a rule and emit the plan.
pub fn plan(rule: &RuleNode) -> TestPlan {
    let mut branches = Vec::new();
    walk(rule, &mut branches);
    TestPlan { branches }
}

fn walk(rule: &RuleNode, branches: &mut Vec<LeafBranch>) {
    match rule {
        RuleNode::RejectIf {
            rule, condition, ..
        } => {
            branches.push(LeafBranch {
                rule: *rule,
                kind: BranchKind::Reject,
                condition: render_condition(condition),
            });
        }
        RuleNode::EscalateIf {
            rule, condition, ..
        } => {
            branches.push(LeafBranch {
                rule: *rule,
                kind: BranchKind::Escalate,
                condition: render_condition(condition),
            });
        }
        RuleNode::Given { then, .. } | RuleNode::Bind { then, .. } => walk(then, branches),
        RuleNode::All(rules) | RuleNode::Any(rules) => {
            for sub in rules {
                walk(sub, branches);
            }
        }
    }
}

fn render_condition(expr: &InnerExpr) -> String {
    // Reuse the markdown layer's expression rendering for consistency.
    use crate::policy::backends::markdown;
    markdown::render(&RuleNode::RejectIf {
        rule: RuleId::new("__"),
        condition: expr.clone(),
        reason: crate::policy::Reason::literal(""),
    })
    .lines()
    .next()
    .and_then(|line| line.split_once("when `"))
    .and_then(|(_, rest)| rest.split_once("` —"))
    .map_or_else(|| "<expr>".into(), |(left, _)| left.to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::policy::ast::{BoolT, Expr};
    use crate::policy::reason::Reason;

    #[test]
    fn plan_lists_one_branch_per_leaf() {
        let rule = RuleNode::All(vec![
            RuleNode::RejectIf {
                rule: RuleId::new("a"),
                condition: Expr::<BoolT>::lit(true).into_inner(),
                reason: Reason::literal("a"),
            },
            RuleNode::EscalateIf {
                rule: RuleId::new("b"),
                condition: Expr::<BoolT>::lit(false).into_inner(),
                to: crate::policy::EscalationTarget::new("desk"),
                reason: Reason::literal("b"),
            },
        ]);
        let p = plan(&rule);
        assert_eq!(p.branches.len(), 2);
        assert_eq!(p.branches[0].rule, RuleId::new("a"));
        assert_eq!(p.branches[0].kind, BranchKind::Reject);
        assert_eq!(p.branches[1].rule, RuleId::new("b"));
        assert_eq!(p.branches[1].kind, BranchKind::Escalate);
    }
}
