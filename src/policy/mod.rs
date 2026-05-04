//! The nuke policy eDSL.
//!
//! See `ROADMAP.md` for the full design. This module is the writing
//! surface; the AST and backends will land here as separate epics.
//!
//! Currently lives:
//! - [`Decision`] — total verdict algebra (`Allow | Deny | Escalate`).
//! - [`Reason`] — typed format-string AST (named slots, never `String`).
//! - [`Bindings`] — values captured during evaluation, attached to
//!   `Deny`/`Escalate` so verdicts are self-explanatory and reproducible.
//! - [`RuleId`] — interned `&'static str` rule identifier.

pub mod ast;
pub mod capability;
mod decision;
pub mod eval;
mod reason;
pub mod registry;

pub use capability::{Capability, Context, HasInventory, HasMarketData, HasOrder, HasRiskLimits};

pub use ast::{
    BinOp, BinOpExpr, BoolT, CmpExpr, CmpOp, Comparable, DecT, Expr, ExprType, FieldRef, InnerExpr,
    LitValue, NotionalT, Numeric, PxT, QtyT, RuleNode, SideT, SymbolT, TextT, TypeTag, add, eq,
    field, ge, gt, le, lt, ne, notional_div_px, qty_times_px, sub,
};
pub use decision::{Decision, EscalationTarget, RuleId};
pub use reason::{Bindings, Reason, ReasonChunk, ReasonTemplate, Slot, SlotName, SlotValue};
