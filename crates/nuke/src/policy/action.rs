//! The [`Action`] trait + DAG-construction surface.
//!
//! A policy is verdict logic plus *side-effect verbs*. Verbs (Buy,
//! Sell, Short, Transfer, Lend, Stake, ...) are the open extension
//! point of the eDSL: adopters add a new verb by implementing
//! [`Action`], not by adding a string label to a registry. Each
//! [`Action`] knows how to lower itself into a sub-DAG of nodes that
//! handle the verb's full lifecycle (construct -> risk-check ->
//! sign -> submit -> wait-for-fill -> emit-event-or-error). The
//! framework's compiler walks the policy AST, calls [`Action::lower`]
//! at each [`crate::policy::ast::RuleNode::Do`] leaf, and stitches
//! the sub-DAGs together with predicate-gate edges from the
//! surrounding [`crate::policy::ast::RuleNode`] structure.
//!
//! Verbs live in adapter or example crates, never in the framework.
//! The framework ships [`Action`] + the compiler; everything else is
//! adopter code.
//!
//! # Erasure
//!
//! [`RuleNode::Do`](crate::policy::ast::RuleNode::Do) holds a
//! `Box<dyn ErasedAction>` so a single rule tree can mix verbs of
//! different output types. The blanket [`ErasedAction`] impl on
//! every [`Action`] type bridges the typed surface to the trait
//! object the AST stores.

use std::fmt::Debug;
use std::marker::PhantomData;

use crate::policy::backends::dag::{DagNode, DagPlan, Edge, NodeId};

// ---------------------------------------------------------------------
// Public typed surface
// ---------------------------------------------------------------------

/// One verb. Adopters implement [`Action`] for each domain-specific
/// thing a policy can *do* - place an order, transfer funds, stake a
/// validator. The implementation lowers the verb into a sub-DAG of
/// nodes via [`Action::lower`]; the surrounding compiler wires those
/// nodes into the larger policy DAG.
///
/// `KIND` is a stable identifier for this verb (used in audit, golden
/// tests, and schema hashing). `Output` is the type the verb's
/// terminal node yields - downstream nodes (further actions, audit
/// emitters, etc.) depend on a [`NodeHandle<Output>`].
pub trait Action: Debug + Send + Sync + 'static {
    /// Stable identifier for this verb. Two impls with the same KIND
    /// are treated as the same verb by audit / wire / diff backends.
    const KIND: &'static str;

    /// Type of the terminal node the lowering produces.
    type Output: Send + 'static;

    /// Lower this verb into the DAG. Returns the typed handle of the
    /// terminal node so callers (and the compiler) can depend on it.
    /// Implementations are free to add as many internal nodes as
    /// they need.
    fn lower(&self, dag: &mut DagBuilder) -> NodeHandle<Self::Output>;
}

/// Object-safe view of an [`Action`] used in
/// [`RuleNode::Do`](crate::policy::ast::RuleNode::Do). Adopters do
/// not implement this directly - the blanket impl on every
/// [`Action`] handles erasure.
pub trait ErasedAction: Debug + Send + Sync + 'static {
    /// Forwarded from [`Action::KIND`].
    fn kind(&self) -> &'static str;
    /// Lower this action into `dag`, returning the terminal node id.
    /// Type information is erased at this boundary; downstream nodes
    /// referring to the result handle do so by id.
    fn lower_erased(&self, dag: &mut DagBuilder) -> NodeId;
    /// Deep copy. Required because trait objects are not `Clone`,
    /// and the AST [`RuleNode`] needs `Clone` for backend folds that
    /// transform sub-trees.
    fn clone_box(&self) -> Box<dyn ErasedAction>;
}

impl<A: Action + Clone> ErasedAction for A {
    fn kind(&self) -> &'static str {
        A::KIND
    }

    fn lower_erased(&self, dag: &mut DagBuilder) -> NodeId {
        self.lower(dag).id
    }

    fn clone_box(&self) -> Box<dyn ErasedAction> {
        Box::new(self.clone())
    }
}

impl Clone for Box<dyn ErasedAction> {
    fn clone(&self) -> Self {
        self.clone_box()
    }
}

impl PartialEq for Box<dyn ErasedAction> {
    /// Trait-object equality is intentionally weak: two actions are
    /// "equal" iff they share a KIND. This is enough for the
    /// semantic-diff backend (adding/removing a verb is a
    /// detectable change) without forcing every Action to be
    /// `PartialEq`. Equality on the typed payload is the verb
    /// implementor's concern (e.g., compare two `Buy<V>` values
    /// directly via `Action`-aware code).
    fn eq(&self, other: &Self) -> bool {
        self.kind() == other.kind()
    }
}

impl Eq for Box<dyn ErasedAction> {}

impl serde::Serialize for Box<dyn ErasedAction> {
    /// Wire-encode an action as just its KIND. The schema-hash
    /// backend uses this to detect "the verb at this leaf changed";
    /// it intentionally does not capture the action's payload, since
    /// payloads are typed at the verb impl and not framework-known.
    fn serialize<S: serde::Serializer>(&self, ser: S) -> Result<S::Ok, S::Error> {
        ser.serialize_str(self.kind())
    }
}

// ---------------------------------------------------------------------
// DAG construction surface
// ---------------------------------------------------------------------

/// Phantom-typed handle to a node in a [`DagBuilder`]. The phantom
/// `T` is the type the node produces - downstream nodes can encode
/// "I depend on a node that yields `T`" at the type level even
/// though the underlying DAG storage is untyped.
#[derive(Debug)]
pub struct NodeHandle<T> {
    pub id: NodeId,
    _phantom: PhantomData<fn() -> T>,
}

impl<T> NodeHandle<T> {
    /// Wrap a raw [`NodeId`] in a typed handle. Crate-internal
    /// helper for the policy compiler when stitching framework-known
    /// node kinds (Guard, Predicate, Bind, Combinator) - those
    /// don't carry meaningful output types, so the compiler uses
    /// `NodeHandle<()>` to reuse the typed edge API.
    pub(crate) fn from_id(id: NodeId) -> Self {
        Self {
            id,
            _phantom: PhantomData,
        }
    }
}

impl<T> Clone for NodeHandle<T> {
    fn clone(&self) -> Self {
        *self
    }
}

impl<T> Copy for NodeHandle<T> {}

/// Mutable builder threaded through [`Action::lower`]. Wraps a
/// growing [`DagPlan`]; expose only the operations a verb impl
/// should need (add a node, add an edge, finalize).
#[derive(Debug, Default)]
pub struct DagBuilder {
    plan: DagPlan,
}

impl DagBuilder {
    pub fn new() -> Self {
        Self::default()
    }

    /// Add a node and return its typed handle.
    pub fn add_node<T>(&mut self, node: DagNode) -> NodeHandle<T> {
        let id = NodeId(self.plan.nodes.len());
        self.plan.nodes.push(node);
        NodeHandle {
            id,
            _phantom: PhantomData,
        }
    }

    /// Add a parent->child edge between two existing nodes.
    pub fn depend<P, C>(&mut self, parent: NodeHandle<P>, child: NodeHandle<C>) {
        self.plan.edges.push(Edge {
            from: parent.id,
            to: child.id,
        });
    }

    /// Set this DAG's entry node. Called once by the policy compiler
    /// after the root walk completes.
    pub fn set_root(&mut self, root: NodeId) {
        self.plan.root = root;
    }

    /// Borrow the in-progress plan. Useful for backends that need to
    /// inspect the current state mid-walk (rare).
    pub fn plan(&self) -> &DagPlan {
        &self.plan
    }

    /// Consume the builder and yield the finished plan.
    pub fn finish(self) -> DagPlan {
        self.plan
    }
}
