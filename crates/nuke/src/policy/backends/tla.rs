//! TLA+ export - emits a TLA+ snippet for the rule's deny condition.
//! Used when a policy participates in a larger order-lifecycle state
//! machine and you want the combined model checked.
//!
//! V0 produces a TLA+ predicate definition; the surrounding spec
//! (variables, init, next-state) is the operator's responsibility.

use std::fmt::Write;

use crate::policy::ast::{
    BinOp, BinOpExpr, CmpExpr, CmpOp, FieldRef, InnerExpr, LitValue, RuleNode,
};

/// Render the rule as a TLA+ predicate operator. The caller invokes
/// it as `<predicate_name>(state)` from the surrounding spec.
pub fn render<A>(predicate_name: &str, rule: &RuleNode<A>) -> String {
    let mut out = String::new();
    writeln!(out, "(* nuke TLA+ export for `{predicate_name}` *)").ok();
    writeln!(out, "{predicate_name}(state) ==").ok();
    writeln!(out, "  {}", rule_to_tla(rule)).ok();
    out
}

fn rule_to_tla<A>(rule: &RuleNode<A>) -> String {
    match rule {
        RuleNode::Given { conditions, then } => {
            let guard = conditions
                .iter()
                .map(expr_to_tla)
                .collect::<Vec<_>>()
                .join(" /\\ ");
            format!("({guard}) => ({})", rule_to_tla(then))
        }
        RuleNode::RejectIf { condition, .. } | RuleNode::EscalateIf { condition, .. } => {
            expr_to_tla(condition)
        }
        RuleNode::All(rules) => rules
            .iter()
            .map(rule_to_tla)
            .collect::<Vec<_>>()
            .join(" /\\ "),
        RuleNode::Any(rules) => rules
            .iter()
            .map(rule_to_tla)
            .collect::<Vec<_>>()
            .join(" \\/ "),
        RuleNode::Bind { then, .. } => rule_to_tla(then),
        // `Run` is not a logical assertion. It contributes `TRUE` to
        // the predicate so the surrounding conjunction / disjunction
        // structure stays well-formed.
        RuleNode::Do(_) => "TRUE".to_string(),
    }
}

fn expr_to_tla(expr: &InnerExpr) -> String {
    match expr {
        InnerExpr::Lit(value) => lit_to_tla(value),
        InnerExpr::Field(FieldRef { entity, name }) => format!("state.{entity}.{name}"),
        InnerExpr::Slot(name) => format!("slot_{name}"),
        InnerExpr::Cmp(CmpExpr { op, lhs, rhs }) => {
            format!(
                "({} {} {})",
                expr_to_tla(lhs),
                cmp_to_tla(*op),
                expr_to_tla(rhs)
            )
        }
        InnerExpr::BinOp(BinOpExpr { op, lhs, rhs }) => {
            format!(
                "({} {} {})",
                expr_to_tla(lhs),
                bin_to_tla(*op),
                expr_to_tla(rhs)
            )
        }
        InnerExpr::Not(inner) => format!("\\lnot ({})", expr_to_tla(inner)),
        InnerExpr::And(parts) => parts
            .iter()
            .map(expr_to_tla)
            .collect::<Vec<_>>()
            .join(" /\\ "),
        InnerExpr::Or(parts) => parts
            .iter()
            .map(expr_to_tla)
            .collect::<Vec<_>>()
            .join(" \\/ "),
    }
}

fn lit_to_tla(value: &LitValue) -> String {
    match value {
        LitValue::Bool(true) => "TRUE".into(),
        LitValue::Bool(false) => "FALSE".into(),
        LitValue::Decimal(value) => value.to_string(),
        LitValue::Px(value) => value.into_inner().to_string(),
        LitValue::Qty(value) => value.into_inner().to_string(),
        LitValue::Notional(value) => value.into_inner().to_string(),
        LitValue::Symbol(value) => format!("\"{value}\""),
        LitValue::Side(value) => format!("\"{value}\""),
        LitValue::Text(value) => format!("\"{value}\""),
    }
}

const fn cmp_to_tla(op: CmpOp) -> &'static str {
    match op {
        CmpOp::Eq => "=",
        CmpOp::Ne => "/=",
        CmpOp::Lt => "<",
        CmpOp::Le => "<=",
        CmpOp::Gt => ">",
        CmpOp::Ge => ">=",
    }
}

const fn bin_to_tla(op: BinOp) -> &'static str {
    match op {
        BinOp::Add => "+",
        BinOp::Sub => "-",
        BinOp::Mul => "*",
        BinOp::Div => "\\div",
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
    fn renders_predicate_operator_with_field_access() {
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
        let tla = render("WouldReject", &rule);
        assert!(tla.contains("WouldReject(state) =="), "got: {tla}");
        assert!(tla.contains("state.order.qty"), "got: {tla}");
        assert!(tla.contains("> 100"), "got: {tla}");
    }
}
