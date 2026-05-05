//! Declarative writing surface for the eDSL.
//!
//! Every macro here desugars to constructors over the typed AST in
//! [`crate::policy::ast`]. The fixed grammar - `reject_when!`,
//! `escalate_when!`, `all_of!`, `any_of!`, `given!`, `bind_as!`,
//! `define_rule!` - is the syntactic firewall against free-form Rust
//! blocks: the `condition` slot must implement `into_inner()` (i.e. be
//! a typed `Expr<BoolT>`), so a stray closure or arbitrary `if`
//! expression won't even compile.
//!
//! The `policy!` macro is the umbrella that selects between the
//! sub-shapes for a single rule.
//!
//! ```ignore
//! use nuke::policy::*;
//! use nuke::{policy, register_rule};
//!
//! register_rule!(RuleId::new("orders.max_size"), "orders/max_size.md");
//!
//! let rule = policy! {
//!     reject "orders.max_size"
//!     when gt(order::qty(), Expr::<QtyT>::lit(Qty::new(dec!(1000))))
//!     because "order qty exceeds limit"
//! };
//! ```

/// Build a [`RuleNode::RejectIf`](crate::policy::ast::RuleNode::RejectIf)
/// from a rule id, a typed `Expr<BoolT>` condition, and a literal reason.
#[macro_export]
macro_rules! reject_when {
    ($id:literal, $condition:expr, $reason:literal $(,)?) => {
        $crate::policy::ast::RuleNode::RejectIf {
            rule: $crate::policy::RuleId::new($id),
            condition: $condition.into_inner(),
            reason: $crate::policy::Reason::literal($reason),
        }
    };
}

/// Build a [`RuleNode::EscalateIf`](crate::policy::ast::RuleNode::EscalateIf).
#[macro_export]
macro_rules! escalate_when {
    ($id:literal, $condition:expr, to $target:literal, because $reason:literal $(,)?) => {
        $crate::policy::ast::RuleNode::EscalateIf {
            rule: $crate::policy::RuleId::new($id),
            condition: $condition.into_inner(),
            to: $crate::policy::EscalationTarget::new($target),
            reason: $crate::policy::Reason::literal($reason),
        }
    };
}

/// Conjunctive combinator: every sub-rule must `Allow`.
#[macro_export]
macro_rules! all_of {
    ( $($sub:expr),+ $(,)? ) => {
        $crate::policy::ast::RuleNode::All(::std::vec![$($sub),+])
    };
}

/// First-deny-wins combinator: walks sub-rules in order; the first
/// non-`Allow` decision is returned.
#[macro_export]
macro_rules! any_of {
    ( $($sub:expr),+ $(,)? ) => {
        $crate::policy::ast::RuleNode::Any(::std::vec![$($sub),+])
    };
}

/// Guard combinator: the inner rule only fires when every condition
/// holds. When a guard fails, the wrapper short-circuits to `Allow`.
#[macro_export]
macro_rules! given {
    ( [ $($condition:expr),+ $(,)? ] then $body:expr $(,)? ) => {
        $crate::policy::ast::RuleNode::Given {
            conditions: ::std::vec![$($condition.into_inner()),+],
            then: ::std::boxed::Box::new($body),
        }
    };
}

/// Bind a captured value into the bindings table under `$name` and
/// then evaluate `$body`. Captured values flow into any downstream
/// `Deny`/`Escalate`'s `bindings`, making rejections self-explanatory.
#[macro_export]
macro_rules! bind_as {
    ( $name:literal = $expr:expr, then $body:expr $(,)? ) => {
        $crate::policy::ast::RuleNode::Bind {
            name: $crate::policy::SlotName($name),
            expr: $expr.into_inner(),
            then: ::std::boxed::Box::new($body),
        }
    };
}

/// Umbrella macro selecting the appropriate sub-shape. Adopters who
/// prefer the fixed phrasing can write `policy! { reject "id" when expr
/// because "..." }` and so on.
#[macro_export]
macro_rules! policy {
    ( reject $id:literal when $condition:expr, because $reason:literal $(,)? ) => {
        $crate::reject_when!($id, $condition, $reason)
    };
    ( escalate $id:literal when $condition:expr, to $target:literal, because $reason:literal $(,)? ) => {
        $crate::escalate_when!($id, $condition, to $target, because $reason)
    };
    ( all $($body:tt)+ ) => {
        $crate::all_of!($($body)+)
    };
    ( any $($body:tt)+ ) => {
        $crate::any_of!($($body)+)
    };
    ( given [ $($condition:expr),+ $(,)? ] then $body:expr $(,)? ) => {
        $crate::given!([$($condition),+] then $body)
    };
    ( bind $name:literal = $expr:expr, then $body:expr $(,)? ) => {
        $crate::bind_as!($name = $expr, then $body)
    };
}

/// Module-position macro that registers a rule's id + markdown path
/// and exposes its `RuleNode` via a generated function.
///
/// ```ignore
/// define_rule! {
///     pub max_order_size,
///     id = "orders.max_size",
///     markdown = "orders/max_size.md",
///     body = reject_when!(
///         "orders.max_size",
///         gt(order::qty(), Expr::<QtyT>::lit(Qty::new(dec!(1000)))),
///         "order qty exceeds limit",
///     ),
/// }
///
/// // Generates:
/// //   #[linkme::distributed_slice(...)] static _ENTRY: RuleEntry = ...;
/// //   pub fn max_order_size() -> RuleNode { ... }
/// ```
#[macro_export]
macro_rules! define_rule {
    (
        $vis:vis $name:ident,
        id = $id:literal,
        markdown = $markdown:literal,
        body = $body:expr $(,)?
    ) => {
        $crate::register_rule!($crate::policy::RuleId::new($id), $markdown);

        #[doc = concat!("Compiled rule `", $id, "` (markdown: `", $markdown, "`).")]
        $vis fn $name() -> $crate::policy::ast::RuleNode {
            $body
        }
    };
}

#[cfg(test)]
mod tests {
    use crate::domain::Qty;
    use crate::policy::ast::{BoolT, Expr, QtyT, gt, lt};
    use crate::policy::capability::{Context, HasOrder};
    use crate::policy::eval::evaluate;
    use crate::policy::reason::SlotValue;
    use rust_decimal::Decimal;

    fn d(value: i64) -> Decimal {
        Decimal::from(value)
    }

    struct OrderCtx {
        qty: Qty,
    }

    impl Context for OrderCtx {
        fn lookup(&self, entity: &str, name: &str) -> Option<SlotValue> {
            match (entity, name) {
                ("order", "qty") => Some(SlotValue::Qty(self.qty)),
                _ => None,
            }
        }
    }

    impl HasOrder for OrderCtx {}

    /// Helper to keep the test bodies readable.
    fn order_qty() -> Expr<QtyT> {
        crate::policy::ast::field::<QtyT>("order", "qty")
    }

    #[test]
    fn reject_when_macro_builds_reject_if_node() {
        let rule = reject_when!(
            "orders.too_small",
            lt(order_qty(), Expr::<QtyT>::lit(Qty::new(d(10)))),
            "order qty too small",
        );
        let ctx = OrderCtx {
            qty: Qty::new(d(5)),
        };
        assert!(evaluate(&rule, &ctx).unwrap().is_deny());
    }

    #[test]
    fn escalate_when_macro_builds_escalate_if_node() {
        let rule = escalate_when!(
            "orders.large",
            gt(order_qty(), Expr::<QtyT>::lit(Qty::new(d(1000)))),
            to "compliance",
            because "large order"
        );
        let ctx = OrderCtx {
            qty: Qty::new(d(5_000)),
        };
        assert!(evaluate(&rule, &ctx).unwrap().is_escalate());
    }

    #[test]
    fn all_of_macro_short_circuits_on_first_deny() {
        let rule = all_of!(
            reject_when!("orders.first", Expr::<BoolT>::lit(false), "ignored",),
            reject_when!("orders.second", Expr::<BoolT>::lit(true), "fires",),
        );
        let ctx = OrderCtx {
            qty: Qty::new(d(1)),
        };
        let decision = evaluate(&rule, &ctx).unwrap();
        assert!(decision.is_deny());
    }

    #[test]
    fn given_short_circuits_to_allow_when_guard_fails() {
        let rule = given!(
            [Expr::<BoolT>::lit(false)] then reject_when!(
                "orders.would_reject",
                Expr::<BoolT>::lit(true),
                "would reject",
            )
        );
        let ctx = OrderCtx {
            qty: Qty::new(d(1)),
        };
        assert!(evaluate(&rule, &ctx).unwrap().is_allow());
    }

    #[test]
    fn policy_umbrella_matches_reject_shape() {
        let rule = policy! {
            reject "orders.too_small"
            when lt(order_qty(), Expr::<QtyT>::lit(Qty::new(d(10)))),
            because "policy macro"
        };
        let ctx = OrderCtx {
            qty: Qty::new(d(5)),
        };
        assert!(evaluate(&rule, &ctx).unwrap().is_deny());
    }
}
