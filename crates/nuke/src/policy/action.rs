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
//!
//! # The gate parameter
//!
//! The policy compiler emits a verdict task per rule and threads its
//! handle through [`PolicyGate`] to every `Action::lower` call. Verbs
//! that should only run on `DecisionTag::Allow` wire their first node
//! to depend on the gate; verbs that ignore it run unconditionally.
//! See [`PolicyGate`] for the wiring pattern.
//!
//! # Bound chain via [`LowerBackend`]
//!
//! `apalis-workflow`'s `add_node` requires a long chain of bounds on
//! the backend type and its codec. Bundling them as a one-line
//! supertrait via [`LowerBackend<I, O>`] keeps verb impls readable:
//! `fn lower<B: LowerBackend<Self::Input, Self::Output>>`. Any
//! backend that satisfies the underlying bounds gets the marker for
//! free via the blanket impl.

use std::fmt::Debug;
use std::future::Future;

use apalis_core::backend::{BackendExt, codec::Codec};
use apalis_core::error::BoxDynError;
use apalis_core::task_fn::into_response::IntoResponse;
use apalis_core::task_fn::task_fn;
use apalis_workflow::DagFlow;
use apalis_workflow::dag::decode::DagCodec;
use apalis_workflow::dag::{NodeBuilder, NodeHandle};

use crate::policy::ctx::PolicyCtx;
use crate::policy::decision::DecisionTag;

/// Bundle of backend bounds an [`Action::lower`] impl needs to call
/// `dag.add_node(...)` for nodes whose input/output types are `I`
/// and `O`. Blanket-implemented for every backend that satisfies the
/// underlying constraints; verb authors don't impl it.
///
/// The third parameter `Err` is the codec error type that both
/// `Codec<I>` and `Codec<O>` must share - apalis-workflow's
/// `add_node` constrains them to be equal so that downstream tasks
/// can convert errors uniformly.
///
/// The constraints are encoded as supertrait *associated-type
/// bounds* (e.g. `BackendExt<Context: Send + Sync + 'static, ...>`)
/// rather than where clauses on the trait def, so they propagate as
/// implied bounds to anywhere `B: LowerBackend<I, O, Err>` is in
/// scope - that's the only way Rust shares constraints across
/// callsites without the unstable `implied_bounds` feature.
pub trait LowerBackend<I, O, Err>:
    BackendExt<
        Context: Send + Sync + 'static,
        IdType: Send + Sync + 'static,
        Codec: Codec<I, Compact = Self::Compact, Error = Err>
                   + Codec<O, Compact = Self::Compact, Error = Err>,
    > + Send
    + Sync
    + 'static
where
    I: Send + Sync + 'static,
    O: Send + Sync + 'static,
    Err: Into<BoxDynError> + Send + 'static,
{
}

impl<B, I, O, Err> LowerBackend<I, O, Err> for B
where
    B: BackendExt + Send + Sync + 'static,
    B::Context: Send + Sync + 'static,
    B::IdType: Send + Sync + 'static,
    B::Codec:
        Codec<I, Compact = B::Compact, Error = Err> + Codec<O, Compact = B::Compact, Error = Err>,
    Err: Into<BoxDynError> + Send + 'static,
    I: Send + Sync + 'static,
    O: Send + Sync + 'static,
{
}

/// Typed handle the policy compiler hands to each [`Action::lower`]
/// call so the verb can wire its first node to depend on the verdict
/// task.
///
/// `PolicyGate` always carries a verdict task's `NodeBuilder` (the
/// verdict is an entry node with no upstream deps, so it stays in
/// builder form). Verb impls call `entry.depends_on(gate.builder())`
/// to wire the dependency; the verb's first node receives a
/// `DecisionTag` value at runtime and is responsible for
/// short-circuiting on `Deny` / `Escalate` (apalis doesn't have a
/// "skip downstream on this value" primitive built in).
pub struct PolicyGate<'a, B>
where
    B: BackendExt,
{
    inner: &'a NodeBuilder<'a, PolicyCtx, DecisionTag, B>,
}

impl<'a, B> PolicyGate<'a, B>
where
    B: BackendExt,
{
    /// Wrap a verdict-task `NodeBuilder` as a gate. Called by the
    /// policy compiler; verb authors don't construct gates.
    pub fn new(verdict: &'a NodeBuilder<'a, PolicyCtx, DecisionTag, B>) -> Self {
        Self { inner: verdict }
    }

    /// The verdict task's `NodeBuilder`. Verb impls pass this to
    /// their first node's `depends_on(...)`.
    pub fn builder(&self) -> &'a NodeBuilder<'a, PolicyCtx, DecisionTag, B> {
        self.inner
    }
}

/// Helper that adds a `task_fn`-based node to the DAG while hiding
/// the apalis-workflow bound chain behind the [`LowerBackend`]
/// marker. Verb impls call this instead of `dag.add_node(...)`
/// directly so they only need `B: LowerBackend<I, O, Err>` plus the
/// associated-type bounds Rust requires at every callsite.
pub fn add_node<'a, B, I, O, Err, F, Fut>(
    dag: &'a DagFlow<B>,
    name: &'a str,
    closure: F,
) -> NodeBuilder<'a, I, O, B>
where
    B: LowerBackend<I, O, Err>,
    B::Context: Send + Sync + 'static,
    B::IdType: Send + Sync + 'static,
    B::Codec: Codec<I, Compact = B::Compact, Error = Err>
        + Codec<O, Compact = B::Compact, Error = Err>
        + 'static,
    Err: Into<BoxDynError> + Send + 'static,
    I: DagCodec<B, Error = Err> + Send + Sync + 'static,
    O: IntoResponse<Output = O> + Send + Sync + 'static,
    F: FnMut(I) -> Fut + Clone + Send + Sync + 'static,
    Fut: Future<Output = O> + Send + 'static,
{
    dag.add_node(name, task_fn(closure))
}

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
    /// don't depend on upstream values (or `DecisionTag` if the verb
    /// gates on the policy verdict via [`PolicyGate`]).
    type Input: Send + Sync + 'static;

    /// Type the verb's terminal node produces. Downstream policy
    /// nodes that depend on this verb's completion see this type.
    type Output: Send + Sync + 'static;

    /// Lower this verb into the DAG. Returns the typed handle of the
    /// terminal node. Implementations are free to add as many
    /// internal nodes as they need via `dag.add_node(...)`.
    ///
    /// `gate` carries the policy verdict task's builder when the
    /// surrounding rule produced one; verbs that should only run on
    /// `Allow` wire their first node to depend on
    /// [`PolicyGate::builder`] (and inspect the `DecisionTag` value
    /// to short-circuit on `Deny` / `Escalate`).
    fn lower<B, Err>(
        &self,
        dag: &DagFlow<B>,
        gate: &PolicyGate<'_, B>,
    ) -> NodeHandle<Self::Input, Self::Output>
    where
        B: LowerBackend<Self::Input, Self::Output, Err>,
        Err: Into<BoxDynError> + Send + 'static;
}

/// No-op verb used as the default action type for action-free
/// policies. `RuleNode<()>` is the natural shape for verdict-only
/// rules (the only side effect is the [`crate::policy::Decision`]).
///
/// In the rare case a rule actually contains `RuleNode::Do(())`, the
/// compiler emits a real (no-op) task so the DAG stays consistent
/// instead of panicking.
impl Action for () {
    const KIND: &'static str = "noop";
    type Input = ();
    type Output = ();

    fn lower<B, Err>(&self, dag: &DagFlow<B>, _gate: &PolicyGate<'_, B>) -> NodeHandle<(), ()>
    where
        B: LowerBackend<(), (), Err>,
        Err: Into<BoxDynError> + Send + 'static,
    {
        // Entry node with `()` input - no upstream deps (the verdict
        // gate has `DecisionTag` output, which doesn't unify with the
        // verb's `()` input). `depends_on(())` is the apalis-workflow
        // idiom for "convert a NodeBuilder with `()` input into a
        // NodeHandle without any dependencies".
        add_node(dag, "noop", |()| async move {}).depends_on(())
    }
}
