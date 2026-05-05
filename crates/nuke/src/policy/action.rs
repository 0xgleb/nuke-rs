//! The [`Action`] trait - the open extension point of the eDSL.
//!
//! A policy is verdict logic plus *side-effect verbs*. Verbs (Buy,
//! Sell, Short, Transfer, Lend, Stake, ...) are how adopters extend
//! the DSL: each verb is a typed value that knows how to lower itself
//! into a sub-DAG of `apalis_workflow::DagFlow` nodes handling the
//! verb's full lifecycle (construct -> risk-check -> sign -> submit
//! -> wait-for-fill -> emit-event-or-error). The compiler walks the
//! policy AST and calls [`Action::lower`] at each
//! [`crate::policy::ast::RuleNode::Do`] leaf.
//!
//! # Why generic, not object-safe
//!
//! [`Action::lower`] takes a `&apalis_workflow::DagFlow<B>` for some
//! `B: BackendExt`, so the method is generic over `B` and the trait
//! is *not* object-safe. Adopters thread their own action type
//! through [`crate::policy::ast::RuleNode<A>`]; an action-rich
//! strategy typically wraps its verbs in a single enum (e.g.
//! `enum ArbVerb { Buy(Buy<V>), Sell(Sell<V>) }`) and uses
//! `RuleNode<ArbVerb>`. Action-free policies use the default
//! `RuleNode<()>` (where `()` is the no-op action).

use std::fmt::Debug;

use apalis_core::backend::BackendExt;
use apalis_workflow::DagFlow;
use apalis_workflow::dag::NodeHandle;

/// One verb. Adopters implement [`Action`] for each domain-specific
/// thing a policy can *do* - place an order, transfer funds, stake a
/// validator. The implementation lowers the verb into a sub-DAG of
/// nodes via [`Action::lower`]; the surrounding compiler wires the
/// returned terminal handle into the larger policy DAG.
///
/// `KIND` is a stable identifier for this verb (used in audit, golden
/// tests, and rendering). `Input`/`Output` are the typed payloads at
/// the verb's entry / terminal nodes - downstream nodes (further
/// actions, audit emitters, etc.) depend on the typed
/// [`NodeHandle<Input, Output>`].
pub trait Action: Debug + Clone + Send + Sync + 'static {
    /// Stable identifier for this verb. Two impls with the same KIND
    /// are treated as the same verb by audit / rendering / diff
    /// backends.
    const KIND: &'static str;

    /// Type the verb's entry node consumes. Use `()` for verbs that
    /// don't depend on upstream values.
    type Input: Send + Sync + 'static;

    /// Type the verb's terminal node produces. Downstream policy
    /// nodes that depend on this verb's completion see this type.
    type Output: Send + Sync + 'static;

    /// Lower this verb into the DAG. Returns the typed handle of the
    /// terminal node. Implementations are free to add as many
    /// internal nodes as they need via `dag.add_node(...)`.
    fn lower<B: BackendExt>(&self, dag: &DagFlow<B>) -> NodeHandle<Self::Input, Self::Output>;
}

/// No-op verb used as the default action type for action-free
/// policies. `RuleNode<()>` is the natural shape for verdict-only
/// rules (the only side effect is the [`crate::policy::Decision`]).
impl Action for () {
    const KIND: &'static str = "noop";
    type Input = ();
    type Output = ();

    fn lower<B: BackendExt>(&self, _dag: &DagFlow<B>) -> NodeHandle<(), ()> {
        unreachable!(
            "the no-op `()` action is only used as a phantom type for \
             action-free policies and is never reached by the compiler"
        )
    }
}
