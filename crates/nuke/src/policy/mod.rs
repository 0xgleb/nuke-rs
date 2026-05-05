//! The nuke policy eDSL.
//!
//! Contains:
//! - [`Decision`] - total verdict algebra (`Allow | Deny | Escalate`).
//! - [`Reason`] - typed format-string AST (named slots, never `String`).
//! - [`Bindings`] - values captured during evaluation, attached to
//!   `Deny`/`Escalate` so verdicts are self-explanatory and reproducible.
//! - [`RuleId`] - interned `&'static str` rule identifier.

pub mod action;
pub mod ast;
pub mod backends;
pub mod capability;
mod decision;
pub mod eval;
mod macros;
mod reason;
pub mod registry;

pub use action::{Action, DagBuilder, ErasedAction, NodeHandle};
pub use capability::{Capability, Context, HasInventory, HasMarketData, HasOrder, HasRiskLimits};

pub use ast::{
    BinOp, BinOpExpr, BoolT, CmpExpr, CmpOp, Comparable, DecT, Expr, ExprType, FieldRef, InnerExpr,
    LitValue, NotionalT, Numeric, PxT, QtyT, RuleNode, SideT, SymbolT, TextT, TypeTag, add, eq,
    field, ge, gt, le, lt, ne, notional_div_px, qty_times_px, sub,
};
pub use decision::{Decision, EscalationTarget, RuleId};
pub use reason::{Bindings, Reason, ReasonChunk, ReasonTemplate, Slot, SlotName, SlotValue};
