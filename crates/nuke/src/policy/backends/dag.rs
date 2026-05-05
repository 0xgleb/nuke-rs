//! Compile a [`RuleNode`] to a directed acyclic graph of nodes
//! suitable for running on `apalis_workflow::DagFlow`.
//!
//! The compilation walks the AST and emits a [`DagPlan`]: a flat list
//! of [`DagNode`]s plus their parent edges. The plan is intentionally
//! materialization-agnostic - every backend can render it (mermaid
//! visualization, debugging, golden tests, ...) without standing up
//! an apalis worker. A separate materialization layer (in adapter
//! crates or in the example wiring) walks the plan and registers each
//! node with `DagFlow::add_node` against a concrete
//! `apalis_core::backend::BackendExt`.
//!
//! # Mapping
//!
//! - [`RuleNode::RejectIf`] / [`RuleNode::EscalateIf`] - one
//!   [`DagNode::Predicate`] node per leaf. Reaching it with the
//!   condition true emits a terminal verdict.
//! - [`RuleNode::Run`] - one [`DagNode::Action`] node per leaf. Only
//!   reachable on the Allow path; materialization wires it to the
//!   reactor's [`crate::Job`] of matching label.
//! - [`RuleNode::Given`] - one [`DagNode::Guard`] node carrying the
//!   guard conditions, gating its `then` subtree.
//! - [`RuleNode::Bind`] - one [`DagNode::Bind`] node feeding its
//!   `then` subtree.
//! - [`RuleNode::All`] / [`RuleNode::Any`] - one
//!   [`DagNode::Combinator`] per node, with edges to each branch
//!   subtree's entry node.
//!
//! Edges flow parent -> child. The plan's [`DagPlan::root`] is the
//! entry node; sinks (nodes with no outgoing edges) are the
//! terminals. Topological ordering is preserved by the construction
//! order of the [`DagPlan::nodes`] vector.

use crate::policy::ast::{ActionSpec, InnerExpr, RuleNode};
use crate::policy::decision::{EscalationTarget, RuleId};
use crate::policy::reason::{Reason, SlotName};

/// Index into [`DagPlan::nodes`] - opaque handle for an emitted node.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct NodeId(pub usize);

/// Compiled DAG: nodes plus parent->child edges. Self-contained; can
/// be inspected, golden-tested, or materialized into an apalis
/// `DagFlow` independently.
#[derive(Debug, Clone, PartialEq)]
pub struct DagPlan {
    pub nodes: Vec<DagNode>,
    pub edges: Vec<Edge>,
    pub root: NodeId,
}

/// A directed parent->child edge between two nodes in a [`DagPlan`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct Edge {
    pub from: NodeId,
    pub to: NodeId,
}

/// One node in a [`DagPlan`]. Carries enough information for either
/// a renderer (mermaid, dot, debug-print) or a materializer (build
/// an apalis service per node) to do its job without re-walking the
/// AST.
#[derive(Debug, Clone, PartialEq)]
pub enum DagNode {
    /// A guard that gates its child subtree behind one or more
    /// boolean predicates. If any predicate fails at runtime the
    /// guarded subtree short-circuits to Allow.
    Guard { conditions: Vec<InnerExpr> },
    /// A predicate leaf that emits a terminal verdict when its
    /// condition holds. `kind` distinguishes [`Verdict::Reject`] from
    /// [`Verdict::Escalate`].
    Predicate {
        rule: RuleId,
        condition: InnerExpr,
        verdict: Verdict,
        reason: Reason,
    },
    /// A side-effect leaf that enqueues a [`crate::Job`] when reached
    /// without short-circuit. Materializers look up the job by
    /// `spec.label` against the reactor's job registry.
    Action(ActionSpec),
    /// Captures `expr` into the bindings table under `name` so
    /// downstream nodes (predicates, actions, other binds) can
    /// reference it via `InnerExpr::Slot(name)`.
    Bind { name: SlotName, expr: InnerExpr },
    /// A composition node combining multiple branch subtrees. `kind`
    /// distinguishes the semantic ([`CombinatorKind::All`] vs
    /// [`CombinatorKind::Any`]); branch subtree entry nodes are
    /// reachable via the [`DagPlan::edges`] outgoing from this node.
    Combinator { kind: CombinatorKind },
}

/// What a [`DagNode::Predicate`] emits when its condition holds.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Verdict {
    Reject,
    Escalate(EscalationTarget),
}

/// Distinguishes the two combinator semantics. Both sequence their
/// branches; the runtime evaluator currently treats them
/// identically, but several backends (markdown, SMT, this DAG
/// backend) preserve the author's intent for documentation /
/// analysis purposes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum CombinatorKind {
    All,
    Any,
}

/// Walk a [`RuleNode`] and emit a [`DagPlan`].
pub fn compile(rule: &RuleNode) -> DagPlan {
    let mut plan = DagPlan {
        nodes: Vec::new(),
        edges: Vec::new(),
        root: NodeId(0),
    };
    plan.root = walk(rule, &mut plan);
    plan
}

fn walk(rule: &RuleNode, plan: &mut DagPlan) -> NodeId {
    match rule {
        RuleNode::Given { conditions, then } => {
            let id = push(
                plan,
                DagNode::Guard {
                    conditions: conditions.clone(),
                },
            );
            let child = walk(then, plan);
            plan.edges.push(Edge {
                from: id,
                to: child,
            });
            id
        }
        RuleNode::RejectIf {
            rule,
            condition,
            reason,
        } => push(
            plan,
            DagNode::Predicate {
                rule: *rule,
                condition: condition.clone(),
                verdict: Verdict::Reject,
                reason: reason.clone(),
            },
        ),
        RuleNode::EscalateIf {
            rule,
            condition,
            to,
            reason,
        } => push(
            plan,
            DagNode::Predicate {
                rule: *rule,
                condition: condition.clone(),
                verdict: Verdict::Escalate(*to),
                reason: reason.clone(),
            },
        ),
        RuleNode::All(branches) | RuleNode::Any(branches) => {
            let kind = match rule {
                RuleNode::All(_) => CombinatorKind::All,
                RuleNode::Any(_) => CombinatorKind::Any,
                _ => unreachable!(),
            };
            let id = push(plan, DagNode::Combinator { kind });
            let child_ids: Vec<NodeId> = branches.iter().map(|sub| walk(sub, plan)).collect();
            plan.edges
                .extend(child_ids.into_iter().map(|to| Edge { from: id, to }));
            id
        }
        RuleNode::Bind { name, expr, then } => {
            let id = push(
                plan,
                DagNode::Bind {
                    name: *name,
                    expr: expr.clone(),
                },
            );
            let child = walk(then, plan);
            plan.edges.push(Edge {
                from: id,
                to: child,
            });
            id
        }
        RuleNode::Run(spec) => push(plan, DagNode::Action(spec.clone())),
    }
}

fn push(plan: &mut DagPlan, node: DagNode) -> NodeId {
    let id = NodeId(plan.nodes.len());
    plan.nodes.push(node);
    id
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Label;
    use crate::domain::Qty;
    use crate::policy::ast::{Expr, QtyT, field, gt, lt};
    use crate::policy::reason::Reason;
    use rust_decimal::Decimal;

    fn d(value: i64) -> Decimal {
        Decimal::from(value)
    }

    #[test]
    fn single_reject_yields_one_predicate_node() {
        let rule = RuleNode::RejectIf {
            rule: RuleId::new("test.r"),
            condition: lt(
                field::<QtyT>("order", "qty"),
                Expr::<QtyT>::lit(Qty::new(d(10))),
            )
            .into_inner(),
            reason: Reason::literal("too small"),
        };

        let plan = compile(&rule);

        assert_eq!(plan.nodes.len(), 1);
        assert_eq!(plan.edges.len(), 0);
        assert_eq!(plan.root, NodeId(0));
        assert!(matches!(
            &plan.nodes[0],
            DagNode::Predicate {
                verdict: Verdict::Reject,
                ..
            }
        ));
    }

    #[test]
    fn given_then_run_yields_guard_then_action_with_one_edge() {
        let rule = RuleNode::Given {
            conditions: vec![
                gt(
                    field::<QtyT>("order", "qty"),
                    Expr::<QtyT>::lit(Qty::new(d(0))),
                )
                .into_inner(),
            ],
            then: Box::new(RuleNode::Run(ActionSpec::new(
                Label::new("submit"),
                vec![SlotName("notional")],
            ))),
        };

        let plan = compile(&rule);

        assert_eq!(plan.nodes.len(), 2);
        assert_eq!(plan.edges.len(), 1);
        assert_eq!(plan.root, NodeId(0));
        assert!(matches!(&plan.nodes[0], DagNode::Guard { .. }));
        assert!(matches!(&plan.nodes[1], DagNode::Action(_)));
        assert_eq!(
            plan.edges[0],
            Edge {
                from: NodeId(0),
                to: NodeId(1)
            }
        );
    }

    #[test]
    fn all_combinator_fans_out_to_each_branch() {
        let r1 = RuleNode::RejectIf {
            rule: RuleId::new("test.first"),
            condition: lt(
                field::<QtyT>("order", "qty"),
                Expr::<QtyT>::lit(Qty::new(d(0))),
            )
            .into_inner(),
            reason: Reason::literal("first"),
        };
        let r2 = RuleNode::Run(ActionSpec::new(Label::new("act"), vec![]));
        let rule = RuleNode::All(vec![r1, r2]);

        let plan = compile(&rule);

        assert!(matches!(
            &plan.nodes[0],
            DagNode::Combinator {
                kind: CombinatorKind::All
            }
        ));
        let outgoing: Vec<_> = plan.edges.iter().filter(|e| e.from == NodeId(0)).collect();
        assert_eq!(outgoing.len(), 2, "All node must fan out to both branches");
    }

    #[test]
    fn run_node_carries_action_spec_with_captures() {
        let rule = RuleNode::Run(ActionSpec::new(
            Label::new("emit"),
            vec![SlotName("a"), SlotName("b")],
        ));

        let plan = compile(&rule);

        match &plan.nodes[0] {
            DagNode::Action(spec) => {
                assert_eq!(spec.label, Label::new("emit"));
                assert_eq!(spec.captures, vec![SlotName("a"), SlotName("b")]);
            }
            other => panic!("expected Action, got {other:?}"),
        }
    }
}
