//! SQL backtest backend - compiles a `RuleNode`'s deny condition into
//! a SQL `WHERE` clause. Run against historical flow to measure
//! hit-rate and trader impact before deploying a rule.
//!
//! Field references render as `<entity>.<name>` (table-qualified column
//! names); the caller decides whether to map entities to actual SQL
//! tables or to a single denormalized view.

use crate::policy::ast::{
    BinOp, BinOpExpr, CmpExpr, CmpOp, FieldRef, InnerExpr, LitValue, RuleNode,
};

/// Render the rule as a SQL `WHERE` predicate that matches every row
/// the rule would have denied.
pub fn render_where<A>(rule: &RuleNode<A>) -> String {
    rule_to_sql(rule)
}

fn rule_to_sql<A>(rule: &RuleNode<A>) -> String {
    match rule {
        RuleNode::Given { conditions, then } => {
            let guard = conditions
                .iter()
                .map(expr_to_sql)
                .collect::<Vec<_>>()
                .join(" AND ");
            // `Given X then Reject(Y)` row matches iff `X AND Y`.
            format!("({guard}) AND ({})", rule_to_sql(then))
        }
        RuleNode::RejectIf { condition, .. } | RuleNode::EscalateIf { condition, .. } => {
            expr_to_sql(condition)
        }
        RuleNode::All(rules) => {
            let parts: Vec<String> = rules.iter().map(rule_to_sql).collect();
            format!("({})", parts.join(" OR "))
        }
        RuleNode::Any(rules) => {
            let parts: Vec<String> = rules.iter().map(rule_to_sql).collect();
            format!("({})", parts.join(" OR "))
        }
        RuleNode::Bind { then, .. } => rule_to_sql(then),
        // A `Run` leaf has no row-matching predicate. SQL backends
        // pull the rows that *would* trigger a verdict; an action
        // node carries no condition of its own, so it contributes
        // `TRUE` (any row at this branch's gates fires the action).
        RuleNode::Do(_) => "TRUE".to_string(),
    }
}

fn expr_to_sql(expr: &InnerExpr) -> String {
    match expr {
        InnerExpr::Lit(value) => lit_to_sql(value),
        InnerExpr::Field(FieldRef { entity, name }) => format!("{entity}.{name}"),
        InnerExpr::Slot(name) => format!("/* slot:{name} */ NULL"),
        InnerExpr::Cmp(CmpExpr { op, lhs, rhs }) => {
            format!(
                "({} {} {})",
                expr_to_sql(lhs),
                cmp_to_sql(*op),
                expr_to_sql(rhs)
            )
        }
        InnerExpr::BinOp(BinOpExpr { op, lhs, rhs }) => {
            format!(
                "({} {} {})",
                expr_to_sql(lhs),
                bin_to_sql(*op),
                expr_to_sql(rhs)
            )
        }
        InnerExpr::Not(inner) => format!("NOT ({})", expr_to_sql(inner)),
        InnerExpr::And(parts) => {
            format!(
                "({})",
                parts
                    .iter()
                    .map(expr_to_sql)
                    .collect::<Vec<_>>()
                    .join(" AND ")
            )
        }
        InnerExpr::Or(parts) => {
            format!(
                "({})",
                parts
                    .iter()
                    .map(expr_to_sql)
                    .collect::<Vec<_>>()
                    .join(" OR ")
            )
        }
        InnerExpr::If(node) => format!(
            "(CASE WHEN {} THEN {} ELSE {} END)",
            expr_to_sql(&node.cond),
            expr_to_sql(&node.then),
            expr_to_sql(&node.otherwise),
        ),
    }
}

fn lit_to_sql(value: &LitValue) -> String {
    match value {
        LitValue::Bool(true) => "TRUE".into(),
        LitValue::Bool(false) => "FALSE".into(),
        LitValue::Decimal(value) => value.to_string(),
        LitValue::Px(value) => value.into_inner().to_string(),
        LitValue::Qty(value) => value.into_inner().to_string(),
        LitValue::Notional(value) => value.into_inner().to_string(),
        LitValue::Symbol(value) => format!("'{value}'"),
        LitValue::Side(value) => format!("'{value}'"),
        LitValue::Text(value) => format!("'{value}'"),
    }
}

const fn cmp_to_sql(op: CmpOp) -> &'static str {
    match op {
        CmpOp::Eq => "=",
        CmpOp::Ne => "<>",
        CmpOp::Lt => "<",
        CmpOp::Le => "<=",
        CmpOp::Gt => ">",
        CmpOp::Ge => ">=",
    }
}

const fn bin_to_sql(op: BinOp) -> &'static str {
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
    fn emits_basic_where_clause() {
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
        assert_eq!(render_where(&rule), "(order.qty > 100)");
    }
}
