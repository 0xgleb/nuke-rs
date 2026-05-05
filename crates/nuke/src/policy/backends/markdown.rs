//! Markdown digest backend - renders a `RuleNode` into the
//! human-readable form compliance signs off on. Structural diffs of the
//! generated markdown become free changelogs across rule revisions.

use std::fmt::Write;

use crate::policy::ast::{
    BinOp, BinOpExpr, CmpExpr, CmpOp, FieldRef, InnerExpr, LitValue, RuleNode,
};
use crate::policy::reason::Reason;

/// Render a [`RuleNode`] as a markdown digest.
pub fn render<A: crate::policy::action::Action>(rule: &RuleNode<A>) -> String {
    let mut out = String::new();
    render_rule(rule, 0, &mut out);
    out
}

fn render_rule<A: crate::policy::action::Action>(
    rule: &RuleNode<A>,
    depth: usize,
    out: &mut String,
) {
    let bullet = "  ".repeat(depth);
    match rule {
        RuleNode::Given { conditions, then } => {
            writeln!(out, "{bullet}- **Given** all of:").ok();
            for condition in conditions {
                writeln!(out, "{bullet}  - {}", render_expr(condition)).ok();
            }
            writeln!(out, "{bullet}  then:").ok();
            render_rule(then, depth + 2, out);
        }
        RuleNode::RejectIf {
            rule,
            condition,
            reason,
        } => {
            writeln!(
                out,
                "{bullet}- **Reject** `{rule}` when `{}` - _{}_",
                render_expr(condition),
                render_reason(reason),
            )
            .ok();
        }
        RuleNode::EscalateIf {
            rule,
            condition,
            to,
            reason,
        } => {
            writeln!(
                out,
                "{bullet}- **Escalate** `{rule}` to `{to}` when `{}` - _{}_",
                render_expr(condition),
                render_reason(reason),
            )
            .ok();
        }
        RuleNode::All(rules) => {
            writeln!(out, "{bullet}- **All** of:").ok();
            for sub in rules {
                render_rule(sub, depth + 1, out);
            }
        }
        RuleNode::Any(rules) => {
            writeln!(out, "{bullet}- **Any** of (first deny wins):").ok();
            for sub in rules {
                render_rule(sub, depth + 1, out);
            }
        }
        RuleNode::Bind { name, expr, then } => {
            writeln!(
                out,
                "{bullet}- **Bind** `{}` = `{}`",
                name,
                render_expr(expr)
            )
            .ok();
            render_rule(then, depth, out);
        }
        RuleNode::Do(_) => {
            writeln!(out, "{bullet}- **Do** `{}`", A::KIND).ok();
        }
    }
}

fn render_expr(expr: &InnerExpr) -> String {
    match expr {
        InnerExpr::Lit(value) => render_lit(value),
        InnerExpr::Field(FieldRef { entity, name }) => format!("{entity}.{name}"),
        InnerExpr::Slot(name) => format!("${name}"),
        InnerExpr::Cmp(CmpExpr { op, lhs, rhs }) => {
            format!(
                "({} {} {})",
                render_expr(lhs),
                render_cmp_op(*op),
                render_expr(rhs)
            )
        }
        InnerExpr::BinOp(BinOpExpr { op, lhs, rhs }) => {
            format!(
                "({} {} {})",
                render_expr(lhs),
                render_bin_op(*op),
                render_expr(rhs)
            )
        }
        InnerExpr::Not(inner) => format!("!{}", render_expr(inner)),
        InnerExpr::And(parts) => parts
            .iter()
            .map(render_expr)
            .collect::<Vec<_>>()
            .join(" AND "),
        InnerExpr::Or(parts) => parts
            .iter()
            .map(render_expr)
            .collect::<Vec<_>>()
            .join(" OR "),
    }
}

fn render_lit(value: &LitValue) -> String {
    match value {
        LitValue::Bool(value) => value.to_string(),
        LitValue::Decimal(value) => value.to_string(),
        LitValue::Symbol(value) => format!("\"{value}\""),
        LitValue::Side(value) => value.to_string(),
        LitValue::Px(value) => format!("Px({value})"),
        LitValue::Qty(value) => format!("Qty({value})"),
        LitValue::Notional(value) => format!("Notional({value})"),
        LitValue::Text(value) => format!("\"{value}\""),
    }
}

const fn render_cmp_op(op: CmpOp) -> &'static str {
    match op {
        CmpOp::Eq => "=",
        CmpOp::Ne => "!=",
        CmpOp::Lt => "<",
        CmpOp::Le => "<=",
        CmpOp::Gt => ">",
        CmpOp::Ge => ">=",
    }
}

const fn render_bin_op(op: BinOp) -> &'static str {
    match op {
        BinOp::Add => "+",
        BinOp::Sub => "-",
        BinOp::Mul => "*",
        BinOp::Div => "/",
    }
}

fn render_reason(reason: &Reason) -> String {
    reason.render()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::Qty;
    use crate::policy::ast::{Expr, QtyT, field, gt};
    use crate::policy::decision::RuleId;
    use rust_decimal::Decimal;

    fn d(value: i64) -> Decimal {
        Decimal::from(value)
    }

    #[test]
    fn renders_reject_if_with_field_and_literal() {
        let condition = gt(
            field::<QtyT>("order", "qty"),
            Expr::<QtyT>::lit(Qty::new(d(100))),
        )
        .into_inner();
        let rule: RuleNode = RuleNode::RejectIf {
            rule: RuleId::new("orders.too_large"),
            condition,
            reason: Reason::literal("order qty exceeds limit"),
        };
        let markdown = render(&rule);
        assert!(
            markdown.contains("**Reject** `orders.too_large`"),
            "got: {markdown}"
        );
        assert!(markdown.contains("order.qty"), "got: {markdown}");
        assert!(markdown.contains("Qty(100)"), "got: {markdown}");
        assert!(
            markdown.contains("order qty exceeds limit"),
            "got: {markdown}"
        );
    }

    #[test]
    fn renders_nested_all_and_any_with_indentation() {
        let inner: RuleNode = RuleNode::RejectIf {
            rule: RuleId::new("inner"),
            condition: Expr::<crate::policy::ast::BoolT>::lit(true).into_inner(),
            reason: Reason::literal("inner reason"),
        };
        let rule = RuleNode::All(vec![RuleNode::Any(vec![inner])]);
        let markdown = render(&rule);
        assert!(markdown.starts_with("- **All** of:"), "got: {markdown}");
        assert!(markdown.contains("**Any** of"), "got: {markdown}");
        // Inner reject should be deeper-indented than the wrapper.
        let inner_index = markdown.find("**Reject**").expect("inner rendered");
        let any_index = markdown.find("**Any**").expect("any rendered");
        assert!(inner_index > any_index, "inner should appear after Any");
    }
}
