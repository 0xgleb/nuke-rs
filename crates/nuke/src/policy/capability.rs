//! Servant-style capability tracking for the eDSL.
//!
//! A *capability* is the set of readable entities a rule needs from its
//! context. Each capability is a Rust trait extending [`Context`]. When
//! you write a rule that reads `order.qty`, the rule statically requires
//! [`HasOrder`]; wiring it against a context that doesn't impl `HasOrder`
//! is a compile error, not a runtime panic.
//!
//! The composition story is just trait bounds: `<C: HasOrder +
//! HasInventory>` requires both, and `A + B` is order-independent
//! (`A + B == B + A` as a bound). No HList machinery needed in v0; the
//! trait-bound system already provides the order-independence the user's
//! design asked for.
//!
//! Bridging the type-erased AST (which references fields by string,
//! `FieldRef { entity, name }`) to the typed context happens through
//! [`Context::lookup`]. Each capability is a *promise* that
//! `lookup(its_entity, its_field)` will return `Some(...)` for any
//! field registered against that entity. The `derive(Domain)` proc-macro
//! (lands with task #19) will generate the per-entity match arms so
//! contexts don't write the `lookup` plumbing by hand.

use crate::policy::reason::SlotValue;

/// Bridge between the type-erased AST and a user's typed context. The
/// runtime evaluator dispatches every `FieldRef { entity, name }` lookup
/// through this trait; capabilities (below) are the *promises* about
/// which `(entity, name)` pairs will succeed.
pub trait Context {
    /// Resolve a field reference to a [`SlotValue`]. `None` means the
    /// field isn't provided by this context — a capability bound on
    /// the call site should make `None` unreachable for fields the
    /// rule actually requires.
    fn lookup(&self, entity: &str, name: &str) -> Option<SlotValue>;
}

/// A capability declares that this context exposes the named entity
/// through [`Context::lookup`] with `entity == Self::ENTITY`. Rules
/// require capabilities via trait bounds; contexts implement them.
///
/// The associated `ENTITY` const lets the markdown / JSON-Schema /
/// SMT backends label the capability without reflecting on the type.
pub trait Capability: Context {
    /// Stable string the AST's `FieldRef::entity` matches against.
    const ENTITY: &'static str;
}

// ---------------------------------------------------------------------
// Canonical capabilities
//
// These are starter examples; downstream applications define their own
// (e.g. `HasPositionLimits`, `HasOnchainQuote`). Each maps one entity
// type and the entity's fields are registered via derive(Domain).
// ---------------------------------------------------------------------

/// Capability: the rule reads fields of an order
/// (`qty`, `side`, `price`, etc.).
pub trait HasOrder: Context {}
impl<C: HasOrder> Capability for C {
    const ENTITY: &'static str = "order";
}

/// Capability: the rule reads inventory state (current position,
/// available size, etc.).
pub trait HasInventory: Context {}

/// Capability: the rule reads market data (last trade, top of book, etc.).
pub trait HasMarketData: Context {}

/// Capability: the rule reads risk limits (per-symbol, per-account, etc.).
pub trait HasRiskLimits: Context {}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::{Px, Qty, Side};
    use rust_decimal::Decimal;

    /// Test context that implements the canonical capabilities.
    struct TestCtx {
        order_qty: Qty,
        order_side: Side,
        order_price: Px,
    }

    impl Context for TestCtx {
        fn lookup(&self, entity: &str, name: &str) -> Option<SlotValue> {
            match (entity, name) {
                ("order", "qty") => Some(SlotValue::Qty(self.order_qty)),
                ("order", "side") => Some(SlotValue::Side(self.order_side)),
                ("order", "price") => Some(SlotValue::Px(self.order_price)),
                _ => None,
            }
        }
    }

    impl HasOrder for TestCtx {}

    /// Compile-time witness that a rule requiring `HasOrder` accepts
    /// any context that impls it. If you remove `HasOrder` from the
    /// bound it still compiles (since we don't actually call ctx in
    /// this synthetic test); the real compile-time check arrives via
    /// `trybuild` once the runtime evaluator lands.
    fn requires_order<C: HasOrder>(_ctx: &C) {}

    #[test]
    fn capability_resolves_fields() {
        let ctx = TestCtx {
            order_qty: Qty::new(Decimal::from(10)),
            order_side: Side::Buy,
            order_price: Px::new(Decimal::from(100)),
        };
        assert_eq!(
            ctx.lookup("order", "qty"),
            Some(SlotValue::Qty(Qty::new(Decimal::from(10)))),
        );
        assert_eq!(
            ctx.lookup("order", "side"),
            Some(SlotValue::Side(Side::Buy)),
        );
        assert_eq!(ctx.lookup("order", "missing"), None);
        assert_eq!(ctx.lookup("nonexistent", "anything"), None);
    }

    #[test]
    fn rule_can_require_capability() {
        let ctx = TestCtx {
            order_qty: Qty::new(Decimal::from(1)),
            order_side: Side::Sell,
            order_price: Px::new(Decimal::from(50)),
        };
        // If `TestCtx: !HasOrder`, this line wouldn't compile.
        requires_order(&ctx);
    }
}
