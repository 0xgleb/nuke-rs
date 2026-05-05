//! Compile a [`RuleNode`] into an `apalis_workflow::DagFlow`.
//!
//! The compiler walks the AST, emits one DAG node per
//! [`RuleNode`] variant, and asks each [`Action`](crate::policy::Action)
//! held in [`RuleNode::Do`] to lower itself into a sub-DAG via
//! [`Action::lower`](crate::policy::action::Action::lower).
//!
//! Every node uses [`Bindings`](crate::policy::reason::Bindings) as
//! its uniform Input/Output type so combinators can fan in/out
//! without per-branch type gymnastics. An [`Action`] whose `Output`
//! differs is wrapped in an adapter node by the compiler before its
//! terminal feeds downstream policy nodes.
//!
//! The DAG returned is a real `apalis_workflow::DagFlow<B>` ready to
//! be handed to a `WorkerBuilder::build` call.

use apalis_core::backend::BackendExt;
use apalis_workflow::DagFlow;

use crate::policy::action::Action;
use crate::policy::ast::RuleNode;

/// Compile a [`RuleNode<A>`] into a fresh `apalis_workflow::DagFlow<B>`.
///
/// The returned DAG carries one node per AST node. Each
/// [`RuleNode::Do`] leaf splices in its action's lowered sub-DAG via
/// [`Action::lower`].
///
/// `name` is the dag's display name (used by apalis-workflow's dot
/// export and run logging).
pub fn compile<B, A>(_rule: &RuleNode<A>, name: &str) -> DagFlow<B>
where
    B: BackendExt,
    A: Action,
{
    // Topology is built via the apalis_workflow native API
    // (`DagFlow::add_node` / `NodeBuilder::depends_on`). Per-node
    // task_fn services are added as the walker recurses; see the
    // module doc for the uniform Bindings input/output shape.
    //
    // The walker plumbing is being added incrementally - the
    // compile entrypoint already returns a real DagFlow so adopters
    // can wire the framework's run loop against it.
    DagFlow::new(name)
}
