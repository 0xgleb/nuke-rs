//! Typed eDSL AST: [`Expr<T>`] and [`RuleNode`].
//!
//! Initial encoding (NOT tagless-final). Two layers:
//!
//! - **`Expr<T>`** - polymorphic typed expression node. The phantom `T`
//!   carries the value type for compile-time type-checking; the inner
//!   [`InnerExpr`] enum is uniformly enumerated so backends can walk it
//!   without generics.
//! - **`RuleNode`** - control flow over predicates: `Given`, `RejectIf`,
//!   `EscalateIf`, `All`, `Any`, `Bind`. The leaves point at typed
//!   `Expr<Bool>` predicates and structured `Reason`s.
//!
//! No closures anywhere. Every comparison, arithmetic op, field access,
//! and combinator is a named AST variant - that's the whole point. A
//! rule containing a `Fn` cannot be rendered, proved, diffed, or
//! compiled to SQL, which destroys the multi-backend story.

use std::fmt::Debug;
use std::marker::PhantomData;

use rust_decimal::Decimal;
use serde::Serialize;

use crate::domain::{Notional, Px, Qty, Side, Symbol};
use crate::policy::decision::{EscalationTarget, RuleId};
use crate::policy::reason::{Reason, SlotName};

// ---------------------------------------------------------------------
// Public: typed expression wrapper + type tags
// ---------------------------------------------------------------------

/// A typed expression that yields a value of type `T` at evaluation time.
///
/// `T` is a *type-level tag* (zero-sized marker), not the runtime value
/// type - the inner [`InnerExpr`] is type-erased so backends fold over a
/// uniform tree without monomorphizing per `T`. The phantom `T` exists
/// solely to make the *constructors* type-safe: you can't pass a
/// `Expr<BoolT>` where a `Expr<DecT>` is required.
pub struct Expr<T: ExprType>(InnerExpr, PhantomData<T>);

impl<T: ExprType> Debug for Expr<T> {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_tuple("Expr")
            .field(&T::TAG)
            .field(&self.0)
            .finish()
    }
}

impl<T: ExprType> Clone for Expr<T> {
    fn clone(&self) -> Self {
        Self(self.0.clone(), PhantomData)
    }
}

impl<T: ExprType> Expr<T> {
    /// Borrow the type-erased inner node. Used by backends.
    pub fn inner(&self) -> &InnerExpr {
        &self.0
    }

    /// Consume into the type-erased inner node.
    pub fn into_inner(self) -> InnerExpr {
        self.0
    }

    fn from_inner(inner: InnerExpr) -> Self {
        Self(inner, PhantomData)
    }
}

/// Marker trait for valid expression result types. Implementors are
/// zero-sized tag structs ([`BoolT`], [`DecT`], etc.).
pub trait ExprType: 'static {
    /// Stable, debuggable name for the type tag.
    const TAG: TypeTag;
}

/// Marker for expression types that admit ordered comparison
/// (`<`, `<=`, `>`, `>=`).
pub trait Comparable: ExprType {}

/// Marker for expression types that admit arithmetic
/// (`+`, `-`). Multiplication and division are typed
/// case-by-case in [`Expr<DecT>`] / [`Expr<QtyT>`] / etc.
pub trait Numeric: ExprType {}

macro_rules! type_tags {
    ($( $name:ident => $tag:ident ),+ $(,)?) => {
        /// Stable, debuggable enumeration of every expression type tag.
        #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
        pub enum TypeTag {
            $( $tag ),+
        }

        $(
            #[doc = concat!("Type tag for `Expr<", stringify!($name), ">`.")]
            #[derive(Debug, Clone, Copy)]
            pub struct $name;

            impl ExprType for $name {
                const TAG: TypeTag = TypeTag::$tag;
            }
        )+
    }
}

type_tags! {
    BoolT      => Bool,
    DecT       => Decimal,
    PxT        => Px,
    QtyT       => Qty,
    NotionalT  => Notional,
    SideT      => Side,
    SymbolT    => Symbol,
    TextT      => Text,
}

impl Comparable for DecT {}
impl Comparable for PxT {}
impl Comparable for QtyT {}
impl Comparable for NotionalT {}

impl Numeric for DecT {}
impl Numeric for PxT {}
impl Numeric for QtyT {}
impl Numeric for NotionalT {}

// ---------------------------------------------------------------------
// Public: typed constructors
// ---------------------------------------------------------------------

impl Expr<BoolT> {
    pub fn lit(value: bool) -> Self {
        Self::from_inner(InnerExpr::Lit(LitValue::Bool(value)))
    }

    pub fn and(parts: Vec<Self>) -> Self {
        Self::from_inner(InnerExpr::And(
            parts.into_iter().map(|expr| expr.0).collect(),
        ))
    }

    pub fn or(parts: Vec<Self>) -> Self {
        Self::from_inner(InnerExpr::Or(
            parts.into_iter().map(|expr| expr.0).collect(),
        ))
    }
}

impl std::ops::Not for Expr<BoolT> {
    type Output = Self;
    fn not(self) -> Self {
        Self::from_inner(InnerExpr::Not(Box::new(self.0)))
    }
}

impl Expr<DecT> {
    pub fn lit(value: Decimal) -> Self {
        Self::from_inner(InnerExpr::Lit(LitValue::Decimal(value)))
    }
}

impl Expr<PxT> {
    pub fn lit(value: Px) -> Self {
        Self::from_inner(InnerExpr::Lit(LitValue::Px(value)))
    }
}

impl Expr<QtyT> {
    pub fn lit(value: Qty) -> Self {
        Self::from_inner(InnerExpr::Lit(LitValue::Qty(value)))
    }
}

impl Expr<NotionalT> {
    pub fn lit(value: Notional) -> Self {
        Self::from_inner(InnerExpr::Lit(LitValue::Notional(value)))
    }
}

impl Expr<SideT> {
    pub fn lit(value: Side) -> Self {
        Self::from_inner(InnerExpr::Lit(LitValue::Side(value)))
    }
}

impl Expr<SymbolT> {
    pub fn lit(value: Symbol) -> Self {
        Self::from_inner(InnerExpr::Lit(LitValue::Symbol(value)))
    }
}

impl Expr<TextT> {
    pub fn lit(value: &'static str) -> Self {
        Self::from_inner(InnerExpr::Lit(LitValue::Text(value)))
    }
}

/// Read a registered field from the evaluation context. The `T` tag
/// must match the field's registered type, but at this layer we trust
/// the callsite to pass the right `T`.
pub fn field<T: ExprType>(entity: &'static str, name: &'static str) -> Expr<T> {
    Expr::from_inner(InnerExpr::Field(FieldRef { entity, name }))
}

/// Equality comparison. Polymorphic over any [`ExprType`] (`Bool` and
/// type-tagged primitives all admit equality).
pub fn eq<T: ExprType>(lhs: Expr<T>, rhs: Expr<T>) -> Expr<BoolT> {
    cmp(CmpOp::Eq, lhs, rhs)
}

/// Inequality comparison.
pub fn ne<T: ExprType>(lhs: Expr<T>, rhs: Expr<T>) -> Expr<BoolT> {
    cmp(CmpOp::Ne, lhs, rhs)
}

/// `<` comparison, restricted to [`Comparable`] types (no booleans).
pub fn lt<T: Comparable>(lhs: Expr<T>, rhs: Expr<T>) -> Expr<BoolT> {
    cmp(CmpOp::Lt, lhs, rhs)
}

/// `<=` comparison.
pub fn le<T: Comparable>(lhs: Expr<T>, rhs: Expr<T>) -> Expr<BoolT> {
    cmp(CmpOp::Le, lhs, rhs)
}

/// `>` comparison.
pub fn gt<T: Comparable>(lhs: Expr<T>, rhs: Expr<T>) -> Expr<BoolT> {
    cmp(CmpOp::Gt, lhs, rhs)
}

/// `>=` comparison.
pub fn ge<T: Comparable>(lhs: Expr<T>, rhs: Expr<T>) -> Expr<BoolT> {
    cmp(CmpOp::Ge, lhs, rhs)
}

/// Same-type addition over [`Numeric`] tags. `Px + Px` is allowed at
/// the AST level.
pub fn add<T: Numeric>(lhs: Expr<T>, rhs: Expr<T>) -> Expr<T> {
    bin_op_same(BinOp::Add, lhs, rhs)
}

pub fn sub<T: Numeric>(lhs: Expr<T>, rhs: Expr<T>) -> Expr<T> {
    bin_op_same(BinOp::Sub, lhs, rhs)
}

/// Typed `Qty * Px = Notional`, mirroring the domain-primitive
/// multiplication.
pub fn qty_times_px(qty: Expr<QtyT>, price: Expr<PxT>) -> Expr<NotionalT> {
    Expr::from_inner(InnerExpr::BinOp(BinOpExpr {
        op: BinOp::Mul,
        lhs: Box::new(qty.0),
        rhs: Box::new(price.0),
    }))
}

/// `Notional / Px = Qty`.
pub fn notional_div_px(notional: Expr<NotionalT>, price: Expr<PxT>) -> Expr<QtyT> {
    Expr::from_inner(InnerExpr::BinOp(BinOpExpr {
        op: BinOp::Div,
        lhs: Box::new(notional.0),
        rhs: Box::new(price.0),
    }))
}

// ---------------------------------------------------------------------
// Public: type-erased inner enum (what backends walk)
// ---------------------------------------------------------------------

/// Type-erased AST node. Every typed [`Expr<T>`] reduces to this enum
/// once the typed-constructor layer has done its job; backends fold
/// over `InnerExpr` directly without caring about `T`.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub enum InnerExpr {
    Lit(LitValue),
    Field(FieldRef),
    Cmp(CmpExpr),
    BinOp(BinOpExpr),
    Not(Box<InnerExpr>),
    And(Vec<InnerExpr>),
    Or(Vec<InnerExpr>),
    /// Read a previously-bound slot value (introduced by
    /// [`RuleNode::Bind`]).
    Slot(SlotName),
}

/// A literal value of any supported domain type.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub enum LitValue {
    Bool(bool),
    Decimal(Decimal),
    Symbol(Symbol),
    Side(Side),
    Px(Px),
    Qty(Qty),
    Notional(Notional),
    Text(&'static str),
}

/// Reference to a registered field on a domain entity. Looked up at
/// eval time through the context.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize)]
pub struct FieldRef {
    pub entity: &'static str,
    pub name: &'static str,
}

/// A binary comparison node (always yields `Bool`).
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct CmpExpr {
    pub op: CmpOp,
    pub lhs: Box<InnerExpr>,
    pub rhs: Box<InnerExpr>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize)]
pub enum CmpOp {
    Eq,
    Ne,
    Lt,
    Le,
    Gt,
    Ge,
}

/// A binary arithmetic node.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct BinOpExpr {
    pub op: BinOp,
    pub lhs: Box<InnerExpr>,
    pub rhs: Box<InnerExpr>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize)]
pub enum BinOp {
    Add,
    Sub,
    Mul,
    Div,
}

// ---------------------------------------------------------------------
// Public: rule-level control flow
// ---------------------------------------------------------------------

/// Control-flow node above the predicate layer. Folds over `RuleNode`
/// evaluate to a [`Decision`](crate::policy::Decision).
#[derive(Debug, Clone, PartialEq, Serialize)]
pub enum RuleNode {
    /// Guard: only evaluate `then` when *every* `condition` holds.
    /// When a guard fails, the rule short-circuits to `Allow` (the
    /// guarded branch isn't applicable).
    Given {
        conditions: Vec<InnerExpr>,
        then: Box<RuleNode>,
    },
    /// If `condition` is true, return `Deny { rule, reason, bindings }`.
    RejectIf {
        rule: RuleId,
        condition: InnerExpr,
        reason: Reason,
    },
    /// If `condition` is true, return
    /// `Escalate { rule, to, reason, bindings }`.
    EscalateIf {
        rule: RuleId,
        condition: InnerExpr,
        to: EscalationTarget,
        reason: Reason,
    },
    /// Conjunction: every sub-rule must `Allow`. First non-`Allow` wins.
    All(Vec<RuleNode>),
    /// Sequence: first sub-rule that returns non-`Allow` wins. If all
    /// sub-rules `Allow`, the whole node `Allow`s.
    Any(Vec<RuleNode>),
    /// Capture `expr`'s value into the bindings table under `name`,
    /// then evaluate `then`. The bound value becomes available via
    /// `InnerExpr::Slot(name)` and lands in `Bindings` on any
    /// downstream `Deny`/`Escalate`.
    Bind {
        name: SlotName,
        expr: InnerExpr,
        then: Box<RuleNode>,
    },
}

// ---------------------------------------------------------------------
// Private helpers
// ---------------------------------------------------------------------

fn cmp<T: ExprType>(op: CmpOp, lhs: Expr<T>, rhs: Expr<T>) -> Expr<BoolT> {
    Expr::from_inner(InnerExpr::Cmp(CmpExpr {
        op,
        lhs: Box::new(lhs.0),
        rhs: Box::new(rhs.0),
    }))
}

fn bin_op_same<T: Numeric>(op: BinOp, lhs: Expr<T>, rhs: Expr<T>) -> Expr<T> {
    Expr::from_inner(InnerExpr::BinOp(BinOpExpr {
        op,
        lhs: Box::new(lhs.0),
        rhs: Box::new(rhs.0),
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use rust_decimal::Decimal;

    fn d(value: i64) -> Decimal {
        Decimal::from(value)
    }

    #[test]
    fn typed_lit_round_trips_through_inner() {
        let expr = Expr::<DecT>::lit(d(42));
        assert!(matches!(expr.inner(), InnerExpr::Lit(LitValue::Decimal(_))));
    }

    #[test]
    fn comparison_yields_bool_expr() {
        let lhs = Expr::<DecT>::lit(d(10));
        let rhs = Expr::<DecT>::lit(d(20));
        let predicate = lt(lhs, rhs);
        assert!(matches!(predicate.inner(), InnerExpr::Cmp(_)));
    }

    #[test]
    fn typed_qty_times_px_produces_notional_expr() {
        let qty = Expr::<QtyT>::lit(Qty::new(d(10)));
        let px = Expr::<PxT>::lit(Px::new(d(7)));
        let notional = qty_times_px(qty, px);
        assert!(matches!(notional.inner(), InnerExpr::BinOp(_)));
    }

    #[test]
    fn rule_node_can_nest() {
        let condition = lt(field::<DecT>("order", "qty"), Expr::<DecT>::lit(d(100)));
        let rule = RuleNode::RejectIf {
            rule: RuleId::new("orders.too_small"),
            condition: condition.into_inner(),
            reason: Reason::literal("order qty too small"),
        };
        assert!(matches!(rule, RuleNode::RejectIf { .. }));
    }

    #[test]
    fn type_tag_is_stable() {
        assert_eq!(BoolT::TAG, TypeTag::Bool);
        assert_eq!(DecT::TAG, TypeTag::Decimal);
        assert_eq!(PxT::TAG, TypeTag::Px);
        assert_eq!(QtyT::TAG, TypeTag::Qty);
        assert_eq!(NotionalT::TAG, TypeTag::Notional);
    }

    #[test]
    fn logical_combinators_compose() {
        let predicate =
            Expr::<BoolT>::and(vec![Expr::<BoolT>::lit(true), !Expr::<BoolT>::lit(false)]);
        assert!(matches!(predicate.inner(), InnerExpr::And(_)));
    }
}
