//! Runtime evaluator: walks a [`RuleNode`] against a typed
//! [`Context`] and produces a [`Decision`].
//!
//! The first eDSL backend. Every other backend (markdown, SMT, SQL, ...)
//! folds over the same AST; the evaluator is just the fold that
//! produces a verdict instead of a document. Bindings are captured as
//! the walk progresses so a `Deny`/`Escalate` carries the actual
//! values that produced the verdict.
//!
//! No type-level magic at this layer - by the time we get here the AST
//! is already type-erased ([`InnerExpr`]), so the evaluator just does a
//! recursive switch on variants. Type safety happened at construction
//! time in [`crate::policy::ast`].

use rust_decimal::Decimal;

use crate::policy::ast::{
    BinOp, BinOpExpr, CmpExpr, CmpOp, FieldRef, InnerExpr, LitValue, RuleNode,
};
use crate::policy::capability::Context;
use crate::policy::decision::Decision;
use crate::policy::reason::{Bindings, SlotValue};

/// Errors the evaluator can produce. Most of these are "the rule
/// references something the context doesn't provide".
#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum EvalError {
    #[error("missing field: {entity}.{name}")]
    MissingField {
        entity: &'static str,
        name: &'static str,
    },
    #[error("unbound slot: {0}")]
    UnboundSlot(&'static str),
    #[error("type mismatch: expected {expected}, got {actual}")]
    TypeMismatch {
        expected: &'static str,
        actual: &'static str,
    },
}

/// Run a rule against a context. Captures evaluation bindings into the
/// returned `Decision` on `Deny`/`Escalate`.
pub fn evaluate<C: Context>(rule: &RuleNode, ctx: &C) -> Result<Decision, EvalError> {
    let mut bindings = Bindings::empty();
    eval_rule(rule, ctx, &mut bindings)
}

fn eval_rule<C: Context>(
    rule: &RuleNode,
    ctx: &C,
    bindings: &mut Bindings,
) -> Result<Decision, EvalError> {
    match rule {
        RuleNode::Given { conditions, then } => {
            for condition in conditions {
                if !eval_bool(condition, ctx, bindings)? {
                    return Ok(Decision::Allow);
                }
            }
            eval_rule(then, ctx, bindings)
        }
        RuleNode::RejectIf {
            rule,
            condition,
            reason,
        } => {
            if eval_bool(condition, ctx, bindings)? {
                Ok(Decision::Deny {
                    rule: *rule,
                    reason: reason.clone(),
                    bindings: bindings.clone(),
                })
            } else {
                Ok(Decision::Allow)
            }
        }
        RuleNode::EscalateIf {
            rule,
            condition,
            to,
            reason,
        } => {
            if eval_bool(condition, ctx, bindings)? {
                Ok(Decision::Escalate {
                    rule: *rule,
                    to: *to,
                    reason: reason.clone(),
                    bindings: bindings.clone(),
                })
            } else {
                Ok(Decision::Allow)
            }
        }
        RuleNode::All(rules) | RuleNode::Any(rules) => {
            // Both `All` and `Any` short-circuit on the first non-Allow
            // verdict - rules within either combinator are sequenced for
            // deterministic ordering, and nothing in v0 distinguishes
            // them. The names exist so the markdown and SMT backends
            // can still differentiate the author's intent.
            for sub in rules {
                let decision = eval_rule(sub, ctx, bindings)?;
                if !decision.is_allow() {
                    return Ok(decision);
                }
            }
            Ok(Decision::Allow)
        }
        RuleNode::Bind { name, expr, then } => {
            let value = eval_expr(expr, ctx, bindings)?;
            bindings.capture(*name, value);
            eval_rule(then, ctx, bindings)
        }
    }
}

fn eval_expr<C: Context>(
    expr: &InnerExpr,
    ctx: &C,
    bindings: &Bindings,
) -> Result<SlotValue, EvalError> {
    match expr {
        InnerExpr::Lit(value) => Ok(lit_to_slot(value)),
        InnerExpr::Field(field_ref) => lookup_field(ctx, field_ref),
        InnerExpr::Slot(name) => bindings
            .get(*name)
            .cloned()
            .ok_or(EvalError::UnboundSlot(name.0)),
        InnerExpr::Cmp(cmp) => Ok(SlotValue::Bool(eval_cmp(cmp, ctx, bindings)?)),
        InnerExpr::BinOp(bin) => eval_bin_op(bin, ctx, bindings),
        InnerExpr::Not(inner) => {
            let value = eval_expr(inner, ctx, bindings)?;
            Ok(SlotValue::Bool(!as_bool(&value)?))
        }
        InnerExpr::And(parts) => {
            for part in parts {
                let value = eval_expr(part, ctx, bindings)?;
                if !as_bool(&value)? {
                    return Ok(SlotValue::Bool(false));
                }
            }
            Ok(SlotValue::Bool(true))
        }
        InnerExpr::Or(parts) => {
            for part in parts {
                let value = eval_expr(part, ctx, bindings)?;
                if as_bool(&value)? {
                    return Ok(SlotValue::Bool(true));
                }
            }
            Ok(SlotValue::Bool(false))
        }
    }
}

fn eval_bool<C: Context>(
    expr: &InnerExpr,
    ctx: &C,
    bindings: &Bindings,
) -> Result<bool, EvalError> {
    let value = eval_expr(expr, ctx, bindings)?;
    as_bool(&value)
}

fn eval_cmp<C: Context>(cmp: &CmpExpr, ctx: &C, bindings: &Bindings) -> Result<bool, EvalError> {
    let lhs = eval_expr(&cmp.lhs, ctx, bindings)?;
    let rhs = eval_expr(&cmp.rhs, ctx, bindings)?;
    Ok(match cmp.op {
        CmpOp::Eq => lhs == rhs,
        CmpOp::Ne => lhs != rhs,
        CmpOp::Lt => as_decimal(&lhs)? < as_decimal(&rhs)?,
        CmpOp::Le => as_decimal(&lhs)? <= as_decimal(&rhs)?,
        CmpOp::Gt => as_decimal(&lhs)? > as_decimal(&rhs)?,
        CmpOp::Ge => as_decimal(&lhs)? >= as_decimal(&rhs)?,
    })
}

fn eval_bin_op<C: Context>(
    bin: &BinOpExpr,
    ctx: &C,
    bindings: &Bindings,
) -> Result<SlotValue, EvalError> {
    let lhs = as_decimal(&eval_expr(&bin.lhs, ctx, bindings)?)?;
    let rhs = as_decimal(&eval_expr(&bin.rhs, ctx, bindings)?)?;
    let result = match bin.op {
        BinOp::Add => lhs + rhs,
        BinOp::Sub => lhs - rhs,
        BinOp::Mul => lhs * rhs,
        BinOp::Div => lhs / rhs,
    };
    Ok(SlotValue::Decimal(result))
}

fn lookup_field<C: Context>(ctx: &C, field_ref: &FieldRef) -> Result<SlotValue, EvalError> {
    ctx.lookup(field_ref.entity, field_ref.name)
        .ok_or(EvalError::MissingField {
            entity: field_ref.entity,
            name: field_ref.name,
        })
}

fn lit_to_slot(value: &LitValue) -> SlotValue {
    match value {
        LitValue::Bool(value) => SlotValue::Bool(*value),
        LitValue::Decimal(value) => SlotValue::Decimal(*value),
        LitValue::Symbol(value) => SlotValue::Symbol(value.clone()),
        LitValue::Side(value) => SlotValue::Side(*value),
        LitValue::Px(value) => SlotValue::Px(*value),
        LitValue::Qty(value) => SlotValue::Qty(*value),
        LitValue::Notional(value) => SlotValue::Notional(*value),
        LitValue::Text(value) => SlotValue::Text(value),
    }
}

fn as_bool(value: &SlotValue) -> Result<bool, EvalError> {
    match value {
        SlotValue::Bool(value) => Ok(*value),
        other => Err(EvalError::TypeMismatch {
            expected: "Bool",
            actual: variant_name(other),
        }),
    }
}

fn as_decimal(value: &SlotValue) -> Result<Decimal, EvalError> {
    match value {
        SlotValue::Decimal(value) => Ok(*value),
        SlotValue::Px(value) => Ok(value.into_inner()),
        SlotValue::Qty(value) => Ok(value.into_inner()),
        SlotValue::Notional(value) => Ok(value.into_inner()),
        other => Err(EvalError::TypeMismatch {
            expected: "Numeric",
            actual: variant_name(other),
        }),
    }
}

const fn variant_name(value: &SlotValue) -> &'static str {
    match value {
        SlotValue::Bool(_) => "Bool",
        SlotValue::Decimal(_) => "Decimal",
        SlotValue::Symbol(_) => "Symbol",
        SlotValue::Side(_) => "Side",
        SlotValue::Px(_) => "Px",
        SlotValue::Qty(_) => "Qty",
        SlotValue::Notional(_) => "Notional",
        SlotValue::Text(_) => "Text",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::{Px, Qty, Side};
    use crate::policy::ast::{DecT, Expr, QtyT, field, ge, lt, qty_times_px};
    use crate::policy::capability::HasOrder;
    use crate::policy::decision::{EscalationTarget, RuleId};
    use crate::policy::reason::{Reason, SlotName};

    struct OrderCtx {
        qty: Qty,
        side: Side,
        price: Px,
    }

    impl Context for OrderCtx {
        fn lookup(&self, entity: &str, name: &str) -> Option<SlotValue> {
            match (entity, name) {
                ("order", "qty") => Some(SlotValue::Qty(self.qty)),
                ("order", "side") => Some(SlotValue::Side(self.side)),
                ("order", "price") => Some(SlotValue::Px(self.price)),
                _ => None,
            }
        }
    }

    impl HasOrder for OrderCtx {}

    fn d(value: i64) -> Decimal {
        Decimal::from(value)
    }

    #[test]
    fn allow_when_no_rule_fires() {
        let rule = RuleNode::RejectIf {
            rule: RuleId::new("test.never"),
            condition: Expr::<crate::policy::ast::BoolT>::lit(false).into_inner(),
            reason: Reason::literal("never"),
        };
        let ctx = OrderCtx {
            qty: Qty::new(d(1)),
            side: Side::Buy,
            price: Px::new(d(100)),
        };
        let decision = evaluate(&rule, &ctx).unwrap();
        assert!(decision.is_allow());
    }

    #[test]
    fn deny_when_predicate_holds() {
        let condition = lt(
            field::<QtyT>("order", "qty"),
            Expr::<QtyT>::lit(Qty::new(d(10))),
        )
        .into_inner();
        let rule = RuleNode::RejectIf {
            rule: RuleId::new("test.too_small"),
            condition,
            reason: Reason::literal("too small"),
        };
        let ctx = OrderCtx {
            qty: Qty::new(d(5)),
            side: Side::Buy,
            price: Px::new(d(100)),
        };
        let decision = evaluate(&rule, &ctx).unwrap();
        assert!(decision.is_deny());
    }

    #[test]
    fn escalate_when_predicate_holds() {
        let condition = ge(
            field::<QtyT>("order", "qty"),
            Expr::<QtyT>::lit(Qty::new(d(1_000))),
        )
        .into_inner();
        let rule = RuleNode::EscalateIf {
            rule: RuleId::new("test.large_order"),
            condition,
            to: EscalationTarget::new("compliance"),
            reason: Reason::literal("large order"),
        };
        let ctx = OrderCtx {
            qty: Qty::new(d(5_000)),
            side: Side::Buy,
            price: Px::new(d(100)),
        };
        let decision = evaluate(&rule, &ctx).unwrap();
        assert!(decision.is_escalate());
    }

    #[test]
    fn given_short_circuits_to_allow_when_guard_fails() {
        let guard = lt(
            field::<QtyT>("order", "qty"),
            Expr::<QtyT>::lit(Qty::new(d(0))),
        )
        .into_inner();
        let rejection = RuleNode::RejectIf {
            rule: RuleId::new("test.would_reject"),
            condition: Expr::<crate::policy::ast::BoolT>::lit(true).into_inner(),
            reason: Reason::literal("would reject"),
        };
        let rule = RuleNode::Given {
            conditions: vec![guard],
            then: Box::new(rejection),
        };
        let ctx = OrderCtx {
            qty: Qty::new(d(5)),
            side: Side::Buy,
            price: Px::new(d(100)),
        };
        let decision = evaluate(&rule, &ctx).unwrap();
        assert!(
            decision.is_allow(),
            "guard should suppress the inner reject"
        );
    }

    #[test]
    fn bindings_carry_through_to_deny() {
        let rule = RuleNode::Bind {
            name: SlotName("requested"),
            expr: field::<QtyT>("order", "qty").into_inner(),
            then: Box::new(RuleNode::RejectIf {
                rule: RuleId::new("test.always"),
                condition: Expr::<crate::policy::ast::BoolT>::lit(true).into_inner(),
                reason: Reason::literal("always"),
            }),
        };
        let ctx = OrderCtx {
            qty: Qty::new(d(42)),
            side: Side::Buy,
            price: Px::new(d(100)),
        };
        let decision = evaluate(&rule, &ctx).unwrap();
        match decision {
            Decision::Deny { bindings, .. } => {
                assert_eq!(
                    bindings.get(SlotName("requested")),
                    Some(&SlotValue::Qty(Qty::new(d(42)))),
                );
            }
            other => panic!("expected Deny, got {other:?}"),
        }
    }

    #[test]
    fn arithmetic_through_qty_times_px() {
        let notional = qty_times_px(
            field::<QtyT>("order", "qty"),
            field::<crate::policy::ast::PxT>("order", "price"),
        );
        let condition = ge(Expr::<DecT>::lit(d(10_000)), Expr::<DecT>::lit(d(0)));
        // Bind notional and then deny (use it via slot to prove eval).
        let rule = RuleNode::Bind {
            name: SlotName("notional"),
            expr: notional.into_inner(),
            then: Box::new(RuleNode::RejectIf {
                rule: RuleId::new("test.use_notional"),
                condition: condition.into_inner(),
                reason: Reason::literal("notional captured"),
            }),
        };
        let ctx = OrderCtx {
            qty: Qty::new(d(10)),
            side: Side::Buy,
            price: Px::new(d(7)),
        };
        let decision = evaluate(&rule, &ctx).unwrap();
        match decision {
            Decision::Deny { bindings, .. } => {
                assert_eq!(
                    bindings.get(SlotName("notional")),
                    Some(&SlotValue::Decimal(d(70))),
                );
            }
            other => panic!("expected Deny, got {other:?}"),
        }
    }

    /// End-to-end exercise of `#[derive(Domain)]` - generated module
    /// `derived_order` exposes typed accessors and `Order::read_field`
    /// drives the `Context` impl below.
    mod derive_domain_smoke {
        use super::*;
        use crate::domain::{Px, Qty, Side};
        use nuke_derive::Domain;

        #[derive(Domain, Clone)]
        pub struct DerivedOrder {
            pub qty: Qty,
            pub side: Side,
            pub price: Px,
        }

        struct DerivedCtx(DerivedOrder);

        impl Context for DerivedCtx {
            fn lookup(&self, entity: &str, name: &str) -> Option<SlotValue> {
                match entity {
                    "derived_order" => self.0.read_field(name),
                    _ => None,
                }
            }
        }

        #[test]
        fn derive_domain_round_trips_through_evaluator() {
            let condition =
                lt(derived_order::qty(), Expr::<QtyT>::lit(Qty::new(d(10)))).into_inner();
            let rule = RuleNode::RejectIf {
                rule: RuleId::new("test.derived"),
                condition,
                reason: Reason::literal("derived"),
            };
            let ctx = DerivedCtx(DerivedOrder {
                qty: Qty::new(d(5)),
                side: Side::Buy,
                price: Px::new(d(100)),
            });
            let decision = evaluate(&rule, &ctx).unwrap();
            assert!(decision.is_deny());
            assert_eq!(DerivedOrder::ENTITY_NAME, "derived_order");
        }
    }

    #[test]
    fn missing_field_yields_eval_error() {
        let rule = RuleNode::RejectIf {
            rule: RuleId::new("test.missing"),
            condition: field::<crate::policy::ast::BoolT>("order", "nonexistent").into_inner(),
            reason: Reason::literal("won't fire"),
        };
        let ctx = OrderCtx {
            qty: Qty::new(d(1)),
            side: Side::Buy,
            price: Px::new(d(100)),
        };
        let result = evaluate(&rule, &ctx);
        assert!(matches!(
            result,
            Err(EvalError::MissingField {
                entity: "order",
                name: "nonexistent"
            }),
        ));
    }
}
