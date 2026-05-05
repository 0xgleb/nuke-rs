//! Semantic diff over `RuleNode`. Classifies a change between two
//! revisions of a rule as `Identical`, `Narrowed` (more conditions ->
//! denies more strictly), `Widened` (fewer conditions -> allows more),
//! or `Unrelated` (structural change beyond either).
//!
//! Better than `git diff` for review of policy changes - text-level
//! diffs can't tell whether a refactor preserved meaning, and they
//! highlight whitespace-only churn the same as material changes.

use crate::policy::ast::{InnerExpr, RuleNode};
use crate::policy::backends::wire::schema_hash;

/// Outcome of comparing two `RuleNode` revisions.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Diff {
    /// Same hash -> byte-identical AST.
    Identical,
    /// `after` requires *more* conditions to deny / accept than `before`.
    /// Strictly fewer inputs deny than before.
    Narrowed,
    /// `after` requires *fewer* conditions / has *more* deny paths than
    /// `before`. Strictly more inputs deny than before.
    Widened,
    /// Structural change beyond a simple narrow/widen - needs human
    /// review.
    Unrelated,
}

/// Classify the difference between two revisions of a rule.
pub fn diff(before: &RuleNode, after: &RuleNode) -> Diff {
    let hash_before = schema_hash(before).expect("AST is always encodable");
    let hash_after = schema_hash(after).expect("AST is always encodable");
    if hash_before == hash_after {
        return Diff::Identical;
    }
    let leaves_before = leaf_count(before);
    let leaves_after = leaf_count(after);
    let preds_before = predicate_complexity(before);
    let preds_after = predicate_complexity(after);

    // Heuristic: more predicates / more leaves = narrower (stricter
    // conjunctive structure); fewer = wider. Identical leaf count but
    // different shape = unrelated.
    if leaves_after == leaves_before {
        if preds_after > preds_before {
            Diff::Narrowed
        } else if preds_after < preds_before {
            Diff::Widened
        } else {
            Diff::Unrelated
        }
    } else if leaves_after > leaves_before && preds_after >= preds_before {
        Diff::Narrowed
    } else if leaves_after < leaves_before && preds_after <= preds_before {
        Diff::Widened
    } else {
        Diff::Unrelated
    }
}

fn leaf_count(rule: &RuleNode) -> usize {
    match rule {
        RuleNode::RejectIf { .. } | RuleNode::EscalateIf { .. } => 1,
        RuleNode::Given { then, .. } | RuleNode::Bind { then, .. } => leaf_count(then),
        RuleNode::All(rules) | RuleNode::Any(rules) => rules.iter().map(leaf_count).sum(),
        // Side-effect leaves count as a leaf for diff purposes too -
        // adding or removing one is a meaningful behavioral change.
        RuleNode::Do(_) => 1,
    }
}

fn predicate_complexity(rule: &RuleNode) -> usize {
    match rule {
        RuleNode::Given { conditions, then } => {
            conditions.iter().map(expr_size).sum::<usize>() + predicate_complexity(then)
        }
        RuleNode::RejectIf { condition, .. } | RuleNode::EscalateIf { condition, .. } => {
            expr_size(condition)
        }
        RuleNode::All(rules) | RuleNode::Any(rules) => rules.iter().map(predicate_complexity).sum(),
        RuleNode::Bind { expr, then, .. } => expr_size(expr) + predicate_complexity(then),
        // `Run` carries no predicate and contributes zero predicate
        // complexity (its captures are just slot references).
        RuleNode::Do(_) => 0,
    }
}

fn expr_size(expr: &InnerExpr) -> usize {
    match expr {
        InnerExpr::Lit(_) | InnerExpr::Field(_) | InnerExpr::Slot(_) => 1,
        InnerExpr::Cmp(cmp) => 1 + expr_size(&cmp.lhs) + expr_size(&cmp.rhs),
        InnerExpr::BinOp(bin) => 1 + expr_size(&bin.lhs) + expr_size(&bin.rhs),
        InnerExpr::Not(inner) => 1 + expr_size(inner),
        InnerExpr::And(parts) | InnerExpr::Or(parts) => {
            1 + parts.iter().map(expr_size).sum::<usize>()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::Qty;
    use crate::policy::ast::{BoolT, Expr, QtyT, field, gt};
    use crate::policy::decision::RuleId;
    use crate::policy::reason::Reason;

    fn reject_if(condition: InnerExpr) -> RuleNode {
        RuleNode::RejectIf {
            rule: RuleId::new("test.r"),
            condition,
            reason: Reason::literal("x"),
        }
    }

    #[test]
    fn identical_rules_diff_to_identical() {
        let condition = gt(
            field::<QtyT>("order", "qty"),
            Expr::<QtyT>::lit(Qty::new(rust_decimal::Decimal::from(100))),
        )
        .into_inner();
        let before = reject_if(condition.clone());
        let after = reject_if(condition);
        assert_eq!(diff(&before, &after), Diff::Identical);
    }

    #[test]
    fn adding_a_predicate_narrows() {
        let single = Expr::<BoolT>::lit(true);
        let double = Expr::<BoolT>::and(vec![Expr::<BoolT>::lit(true), Expr::<BoolT>::lit(true)]);
        let before = reject_if(single.into_inner());
        let after = reject_if(double.into_inner());
        // More expressions -> narrower rule.
        assert_eq!(diff(&before, &after), Diff::Narrowed);
    }
}
