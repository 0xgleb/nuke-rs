//! SMT-LIB backend - emits a `(declare-const ...)` + `(assert ...)`
//! script for Z3/CVC5. Used by CI for totality (every input lands
//! somewhere) and non-subsumption (no rule fully shadowed by another)
//! proofs.
//!
//! V0 emits SMT-LIB v2 text only - no native Z3 binding. The CI script
//! shells out to `z3 -in` (or `cvc5 --lang smt2`) and feeds the output
//! of [`render_assert`].

use std::collections::BTreeSet;
use std::fmt::Write;

use crate::policy::ast::{
    BinOp, BinOpExpr, CmpExpr, CmpOp, FieldRef, InnerExpr, LitValue, RuleNode,
};

/// Emit the SMT-LIB script that asserts the rule's "deny condition" -
/// useful for proving that two rules can't both fire on the same input
/// (non-subsumption).
pub fn render_assert<A>(rule: &RuleNode<A>) -> String {
    let mut declared: BTreeSet<String> = BTreeSet::new();
    collect_decls(rule, &mut declared);

    let mut out = String::new();
    writeln!(out, "; nuke SMT-LIB export").ok();
    writeln!(out, "(set-logic ALL)").ok();
    for name in &declared {
        // V0: every field is declared as Real (rational). Bool fields
        // would need separate handling once `derive(Domain)` feeds
        // type info into this backend.
        writeln!(out, "(declare-const {name} Real)").ok();
    }
    let predicate = rule_to_smt(rule);
    writeln!(out, "(assert {predicate})").ok();
    writeln!(out, "(check-sat)").ok();
    out
}

fn collect_decls<A>(rule: &RuleNode<A>, into: &mut BTreeSet<String>) {
    match rule {
        RuleNode::Given { conditions, then } => {
            for condition in conditions {
                collect_expr_decls(condition, into);
            }
            collect_decls(then, into);
        }
        RuleNode::RejectIf { condition, .. } | RuleNode::EscalateIf { condition, .. } => {
            collect_expr_decls(condition, into);
        }
        RuleNode::All(rules) | RuleNode::Any(rules) => {
            for sub in rules {
                collect_decls(sub, into);
            }
        }
        RuleNode::Bind { expr, then, .. } => {
            collect_expr_decls(expr, into);
            collect_decls(then, into);
        }
        // `Run` references slots already declared by upstream `Bind`s
        // and contributes no new field declarations of its own.
        RuleNode::Do(_) => {}
    }
}

fn collect_expr_decls(expr: &InnerExpr, into: &mut BTreeSet<String>) {
    match expr {
        InnerExpr::Lit(_) | InnerExpr::Slot(_) => {}
        InnerExpr::Field(FieldRef { entity, name }) => {
            into.insert(format!("{entity}.{name}"));
        }
        InnerExpr::Cmp(CmpExpr { lhs, rhs, .. }) | InnerExpr::BinOp(BinOpExpr { lhs, rhs, .. }) => {
            collect_expr_decls(lhs, into);
            collect_expr_decls(rhs, into);
        }
        InnerExpr::Not(inner) => collect_expr_decls(inner, into),
        InnerExpr::And(parts) | InnerExpr::Or(parts) => {
            for part in parts {
                collect_expr_decls(part, into);
            }
        }
        InnerExpr::If(node) => {
            collect_expr_decls(&node.cond, into);
            collect_expr_decls(&node.then, into);
            collect_expr_decls(&node.otherwise, into);
        }
    }
}

fn rule_to_smt<A>(rule: &RuleNode<A>) -> String {
    match rule {
        RuleNode::Given { conditions, then } => {
            let guard = if conditions.len() == 1 {
                expr_to_smt(&conditions[0])
            } else {
                format!(
                    "(and {})",
                    conditions
                        .iter()
                        .map(expr_to_smt)
                        .collect::<Vec<_>>()
                        .join(" ")
                )
            };
            format!("(=> {guard} {})", rule_to_smt(then))
        }
        RuleNode::RejectIf { condition, .. } | RuleNode::EscalateIf { condition, .. } => {
            expr_to_smt(condition)
        }
        RuleNode::All(rules) => {
            format!(
                "(and {})",
                rules.iter().map(rule_to_smt).collect::<Vec<_>>().join(" "),
            )
        }
        RuleNode::Any(rules) => {
            format!(
                "(or {})",
                rules.iter().map(rule_to_smt).collect::<Vec<_>>().join(" "),
            )
        }
        RuleNode::Bind { then, .. } => rule_to_smt(then),
        // `Run` is not a constraint - the SMT proposition for an
        // action node is "true" (this branch is satisfiable iff its
        // upstream gates are).
        RuleNode::Do(_) => "true".to_string(),
    }
}

fn expr_to_smt(expr: &InnerExpr) -> String {
    match expr {
        InnerExpr::Lit(value) => lit_to_smt(value),
        InnerExpr::Field(FieldRef { entity, name }) => format!("|{entity}.{name}|"),
        InnerExpr::Slot(name) => format!("|${name}|"),
        InnerExpr::Cmp(CmpExpr { op, lhs, rhs }) => format!(
            "({} {} {})",
            cmp_to_smt(*op),
            expr_to_smt(lhs),
            expr_to_smt(rhs)
        ),
        InnerExpr::BinOp(BinOpExpr { op, lhs, rhs }) => format!(
            "({} {} {})",
            bin_to_smt(*op),
            expr_to_smt(lhs),
            expr_to_smt(rhs)
        ),
        InnerExpr::Not(inner) => format!("(not {})", expr_to_smt(inner)),
        InnerExpr::And(parts) => format!(
            "(and {})",
            parts.iter().map(expr_to_smt).collect::<Vec<_>>().join(" ")
        ),
        InnerExpr::Or(parts) => format!(
            "(or {})",
            parts.iter().map(expr_to_smt).collect::<Vec<_>>().join(" ")
        ),
        InnerExpr::If(node) => format!(
            "(ite {} {} {})",
            expr_to_smt(&node.cond),
            expr_to_smt(&node.then),
            expr_to_smt(&node.otherwise),
        ),
    }
}

fn lit_to_smt(value: &LitValue) -> String {
    match value {
        LitValue::Bool(value) => value.to_string(),
        LitValue::Decimal(value) => value.to_string(),
        LitValue::Px(value) => value.into_inner().to_string(),
        LitValue::Qty(value) => value.into_inner().to_string(),
        LitValue::Notional(value) => value.into_inner().to_string(),
        LitValue::Symbol(value) => format!("\"{value}\""),
        LitValue::Side(value) => format!("\"{value}\""),
        LitValue::Text(value) => format!("\"{value}\""),
    }
}

const fn cmp_to_smt(op: CmpOp) -> &'static str {
    match op {
        CmpOp::Eq => "=",
        CmpOp::Ne => "distinct",
        CmpOp::Lt => "<",
        CmpOp::Le => "<=",
        CmpOp::Gt => ">",
        CmpOp::Ge => ">=",
    }
}

const fn bin_to_smt(op: BinOp) -> &'static str {
    match op {
        BinOp::Add => "+",
        BinOp::Sub => "-",
        BinOp::Mul => "*",
        BinOp::Div => "/",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::Qty;
    use crate::policy::ast::{Expr, QtyT, field, gt};
    use crate::policy::decision::RuleId;
    use crate::policy::reason::Reason;

    #[test]
    fn emits_smt_with_field_decl_and_assert() {
        let condition = gt(
            field::<QtyT>("order", "qty"),
            Expr::<QtyT>::lit(Qty::new(rust_decimal::Decimal::from(100))),
        )
        .into_inner();
        let rule: RuleNode = RuleNode::RejectIf {
            rule: RuleId::new("orders.too_large"),
            condition,
            reason: Reason::literal("nope"),
        };
        let smt = render_assert(&rule);
        assert!(smt.contains("(declare-const order.qty Real)"), "got: {smt}");
        assert!(smt.contains("(assert (> |order.qty| 100"), "got: {smt}");
        assert!(smt.contains("(check-sat)"), "got: {smt}");
    }
}
