//! Mermaid backend - emits a flowchart of a `RuleNode` for visual
//! review. Helps catch dead branches and surface the rule's overall
//! shape at a glance.

use std::fmt::Write;

use crate::policy::ast::{InnerExpr, RuleNode};
use crate::policy::backends::markdown;

/// Render a [`RuleNode`] as a mermaid `flowchart TD` block.
pub fn render<A: crate::policy::action::Action>(rule: &RuleNode<A>) -> String {
    let mut out = String::new();
    writeln!(out, "flowchart TD").ok();
    let mut counter: usize = 0;
    let entry = next_id(&mut counter);
    writeln!(out, "    {entry}([Start])").ok();
    let exit = walk(rule, entry, &mut counter, &mut out);
    writeln!(out, "    {exit} --> done([Allow])").ok();
    out
}

fn walk<A: crate::policy::action::Action>(
    rule: &RuleNode<A>,
    prev: String,
    counter: &mut usize,
    out: &mut String,
) -> String {
    match rule {
        RuleNode::Given { conditions, then } => {
            let id = next_id(counter);
            let label = conditions
                .iter()
                .map(render_condition)
                .collect::<Vec<_>>()
                .join(" AND ");
            writeln!(out, "    {id}{{{}}}", escape(&format!("when {label}"))).ok();
            writeln!(out, "    {prev} --> {id}").ok();
            walk(then, id, counter, out)
        }
        RuleNode::RejectIf {
            rule, condition, ..
        } => {
            let id = next_id(counter);
            writeln!(
                out,
                "    {id}([{}])",
                escape(&format!("Reject {rule}: {}", render_condition(condition))),
            )
            .ok();
            writeln!(out, "    {prev} --> {id}").ok();
            id
        }
        RuleNode::EscalateIf {
            rule,
            condition,
            to,
            ..
        } => {
            let id = next_id(counter);
            writeln!(
                out,
                "    {id}([{}])",
                escape(&format!(
                    "Escalate {rule} -> {to}: {}",
                    render_condition(condition)
                )),
            )
            .ok();
            writeln!(out, "    {prev} --> {id}").ok();
            id
        }
        RuleNode::All(rules) | RuleNode::Any(rules) => {
            let mut last = prev;
            for sub in rules {
                last = walk(sub, last, counter, out);
            }
            last
        }
        RuleNode::Bind { name, expr, then } => {
            let id = next_id(counter);
            writeln!(
                out,
                "    {id}[{}]",
                escape(&format!("bind ${name} = {}", render_condition(expr))),
            )
            .ok();
            writeln!(out, "    {prev} --> {id}").ok();
            walk(then, id, counter, out)
        }
        RuleNode::Do(_) => {
            let id = next_id(counter);
            writeln!(out, "    {id}[[{}]]", escape(&format!("do {}", A::KIND))).ok();
            writeln!(out, "    {prev} --> {id}").ok();
            id
        }
    }
}

fn next_id(counter: &mut usize) -> String {
    let id = format!("n{counter}");
    *counter += 1;
    id
}

/// Reuse the markdown layer's expression rendering so the mermaid
/// diagram stays in lockstep with the digest.
fn render_condition(expr: &InnerExpr) -> String {
    let rendered = markdown::render(&RuleNode::<()>::RejectIf {
        rule: crate::policy::RuleId::new("__inline__"),
        condition: expr.clone(),
        reason: crate::policy::Reason::literal(""),
    });
    // Extract the `when ...` part from "Reject ... when X -_..._"
    rendered
        .lines()
        .next()
        .and_then(|line| line.split_once("when `"))
        .and_then(|(_, rest)| rest.split_once("` -"))
        .map_or_else(|| rendered.trim().to_owned(), |(left, _)| left.to_owned())
}

fn escape(label: &str) -> String {
    label.replace('"', "&quot;").replace('\n', " ")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::Qty;
    use crate::policy::ast::{Expr, QtyT, field, gt};
    use crate::policy::decision::RuleId;
    use crate::policy::reason::Reason;

    #[test]
    fn renders_flowchart_with_decision_node() {
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
        let mermaid = render(&rule);
        assert!(mermaid.starts_with("flowchart TD"));
        assert!(mermaid.contains("Reject orders.too_large"));
        assert!(mermaid.contains("Allow"));
    }
}
