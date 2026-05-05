//! Compile a [`RuleNode`] to a directed acyclic graph of nodes
//! suitable for running on `apalis_workflow::DagFlow`.
//!
//! The compilation walks the AST and builds a [`DagPlan`] (a flat
//! list of [`DagNode`]s plus parent->child edges) via the
//! [`DagBuilder`](crate::policy::DagBuilder) surface. The plan is
//! materialization-agnostic: every backend can render it (mermaid
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
//! - [`RuleNode::Do`] - the action's
//!   [`Action::lower`](crate::policy::action::Action::lower) is
//!   called, splicing the verb's sub-DAG into the plan. The action's
//!   terminal node is the one downstream nodes depend on.
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
//! terminals.

use crate::policy::action::DagBuilder;
use crate::policy::ast::{InnerExpr, RuleNode};
use crate::policy::decision::{EscalationTarget, RuleId};
use crate::policy::reason::{Reason, SlotName};

/// Index into [`DagPlan::nodes`] - opaque handle for an emitted node.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct NodeId(pub usize);

/// Compiled DAG: nodes plus parent->child edges. Self-contained; can
/// be inspected, golden-tested, or materialized into an apalis
/// `DagFlow` independently.
#[derive(Debug, Clone, Default, PartialEq)]
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
    /// condition holds. `verdict` distinguishes [`Verdict::Reject`]
    /// from [`Verdict::Escalate`].
    Predicate {
        rule: RuleId,
        condition: InnerExpr,
        verdict: Verdict,
        reason: Reason,
    },
    /// Captures `expr` into the bindings table under `name` so
    /// downstream nodes (predicates, actions, other binds) can
    /// reference it via `InnerExpr::Slot(name)`.
    Bind { name: SlotName, expr: InnerExpr },
    /// A composition node combining multiple branch subtrees.
    /// `kind` distinguishes the semantic ([`CombinatorKind::All`]
    /// vs [`CombinatorKind::Any`]); branch subtree entry nodes are
    /// reachable via the [`DagPlan::edges`] outgoing from this node.
    Combinator { kind: CombinatorKind },
    /// One step within an [`Action`](crate::policy::action::Action)'s
    /// lowered sub-DAG. The verb's `KIND` and a step name are
    /// preserved for audit / rendering; the actual execution
    /// behavior is defined by the [`Action`] impl that emitted this
    /// node and is wired in at materialization time.
    ActionStep {
        action_kind: &'static str,
        step: &'static str,
    },
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
    let mut builder = DagBuilder::new();
    let root = walk(rule, &mut builder);
    builder.set_root(root);
    builder.finish()
}

fn walk(rule: &RuleNode, builder: &mut DagBuilder) -> NodeId {
    match rule {
        RuleNode::Given { conditions, then } => {
            let id = builder
                .add_node::<()>(DagNode::Guard {
                    conditions: conditions.clone(),
                })
                .id;
            let child = walk(then, builder);
            push_edge(builder, id, child);
            id
        }
        RuleNode::RejectIf {
            rule,
            condition,
            reason,
        } => {
            builder
                .add_node::<()>(DagNode::Predicate {
                    rule: *rule,
                    condition: condition.clone(),
                    verdict: Verdict::Reject,
                    reason: reason.clone(),
                })
                .id
        }
        RuleNode::EscalateIf {
            rule,
            condition,
            to,
            reason,
        } => {
            builder
                .add_node::<()>(DagNode::Predicate {
                    rule: *rule,
                    condition: condition.clone(),
                    verdict: Verdict::Escalate(*to),
                    reason: reason.clone(),
                })
                .id
        }
        RuleNode::All(branches) | RuleNode::Any(branches) => {
            let kind = match rule {
                RuleNode::All(_) => CombinatorKind::All,
                RuleNode::Any(_) => CombinatorKind::Any,
                _ => unreachable!(),
            };
            let id = builder.add_node::<()>(DagNode::Combinator { kind }).id;
            for sub in branches {
                let child = walk(sub, builder);
                push_edge(builder, id, child);
            }
            id
        }
        RuleNode::Bind { name, expr, then } => {
            let id = builder
                .add_node::<()>(DagNode::Bind {
                    name: *name,
                    expr: expr.clone(),
                })
                .id;
            let child = walk(then, builder);
            push_edge(builder, id, child);
            id
        }
        RuleNode::Do(action) => action.lower_erased(builder),
    }
}

fn push_edge(builder: &mut DagBuilder, from: NodeId, to: NodeId) {
    use crate::policy::action::NodeHandle;
    // The edge API is typed; for compiler-internal stitching the
    // framework-known node kinds carry no meaningful output type,
    // so we reuse the typed API with `NodeHandle<()>`.
    builder.depend(
        NodeHandle::<()>::from_id(from),
        NodeHandle::<()>::from_id(to),
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::Qty;
    use crate::policy::action::{Action, NodeHandle};
    use crate::policy::ast::{Expr, QtyT, field, gt, lt};
    use crate::policy::reason::Reason;
    use rust_decimal::Decimal;

    fn d(value: i64) -> Decimal {
        Decimal::from(value)
    }

    /// Sample Action used in the tests: lowers to two ActionSteps
    /// (construct + submit) connected by an edge.
    #[derive(Debug, Clone)]
    struct Submit;

    impl Action for Submit {
        const KIND: &'static str = "test.submit";
        type Output = ();

        fn lower(&self, dag: &mut DagBuilder) -> NodeHandle<Self::Output> {
            let construct = dag.add_node::<()>(DagNode::ActionStep {
                action_kind: Self::KIND,
                step: "construct",
            });
            let submit = dag.add_node::<()>(DagNode::ActionStep {
                action_kind: Self::KIND,
                step: "submit",
            });
            dag.depend(construct, submit);
            submit
        }
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
    fn given_then_do_splices_action_subdag_under_guard() {
        let rule = RuleNode::Given {
            conditions: vec![
                gt(
                    field::<QtyT>("order", "qty"),
                    Expr::<QtyT>::lit(Qty::new(d(0))),
                )
                .into_inner(),
            ],
            then: Box::new(RuleNode::Do(Box::new(Submit))),
        };

        let plan = compile(&rule);

        // Guard at 0; Submit lowers to 2 ActionSteps with one
        // internal edge; one edge from Guard to action root.
        assert_eq!(plan.nodes.len(), 3);
        assert_eq!(plan.edges.len(), 2);
        assert_eq!(plan.root, NodeId(0));
        assert!(matches!(&plan.nodes[0], DagNode::Guard { .. }));
        assert!(matches!(
            &plan.nodes[1],
            DagNode::ActionStep {
                step: "construct",
                ..
            }
        ));
        assert!(matches!(
            &plan.nodes[2],
            DagNode::ActionStep { step: "submit", .. }
        ));
        // Internal action edge: construct -> submit.
        assert!(
            plan.edges
                .iter()
                .any(|e| e.from == NodeId(1) && e.to == NodeId(2))
        );
        // Stitch edge: Guard -> action terminal (submit).
        assert!(
            plan.edges
                .iter()
                .any(|e| e.from == NodeId(0) && e.to == NodeId(2))
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
        let r2 = RuleNode::Do(Box::new(Submit));
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
    fn do_node_terminal_is_action_terminal() {
        let rule = RuleNode::Do(Box::new(Submit));

        let plan = compile(&rule);

        // The action lowers to construct + submit; the root is
        // the action's *terminal* (submit), not the construct
        // step. (`Action::lower` returns the terminal handle.)
        assert_eq!(plan.nodes.len(), 2);
        assert_eq!(plan.root, NodeId(1));
    }
}
