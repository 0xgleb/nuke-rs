//! [`PolicyCtx`] - serializable snapshot of the values a policy will
//! read at evaluation time.
//!
//! Apalis tasks need their input to be `Serialize + DeserializeOwned`
//! so they can travel through whatever backend (in-memory, file,
//! sql, redis) the worker uses. The framework's [`Context`] trait
//! is method-based and doesn't constrain the implementor's shape, so
//! adopters typically implement it on whatever runtime struct they
//! already have. [`PolicyCtx`] is the bridge: build it once at the
//! entry of the policy DAG by snapshotting field values via the
//! adopter's existing [`Context`] impl, then every downstream
//! predicate / verdict node receives a clone as its typed input.
//!
//! ## v0 limitations
//!
//! - The `Text` variant of [`crate::policy::SlotValue`] is rejected at
//!   snapshot time. The wire-side mirror enum [`SlotValueWire`] doesn't
//!   carry a `Text` arm because the framework's `&'static str`-backed
//!   `Text` value isn't reconstructible from a deserialized payload.
//!   Policies that read text fields stay on the runtime evaluator path.
//! - [`crate::policy::Bindings`] is not snapshotted: bind-using rules
//!   stay folded into the verdict combinator (the predicate nodes the
//!   DAG walker emits never reference `InnerExpr::Slot`).

use std::collections::BTreeMap;

use apalis_core::backend::{BackendExt, codec::Codec};
use apalis_core::error::BoxDynError;
use apalis_core::task_fn::into_response::IntoResponse;
use apalis_workflow::dag::decode::DagCodec;
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};

use crate::domain::{Notional, Px, Qty, Side, Symbol};
use crate::policy::capability::Context;
use crate::policy::reason::SlotValue;

/// Wire-side mirror of [`SlotValue`] - same variants minus `Text`,
/// every payload owns its data so the value round-trips through serde.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum SlotValueWire {
    Bool(bool),
    Decimal(Decimal),
    Symbol(Symbol),
    Side(Side),
    Px(Px),
    Qty(Qty),
    Notional(Notional),
}

/// Failure converting a [`SlotValue`] into the wire mirror at snapshot
/// time. Returned from [`PolicyCtxBuilder::field`] when the adopter's
/// context returns a `Text` value, which the v0 wire format doesn't
/// support.
#[derive(Debug, thiserror::Error)]
pub enum CtxError {
    #[error("text-valued fields are not yet supported in the policy DAG snapshot")]
    UnsupportedText,
}

impl TryFrom<SlotValue> for SlotValueWire {
    type Error = CtxError;

    fn try_from(value: SlotValue) -> Result<Self, Self::Error> {
        match value {
            SlotValue::Bool(value) => Ok(Self::Bool(value)),
            SlotValue::Decimal(value) => Ok(Self::Decimal(value)),
            SlotValue::Symbol(value) => Ok(Self::Symbol(value)),
            SlotValue::Side(value) => Ok(Self::Side(value)),
            SlotValue::Px(value) => Ok(Self::Px(value)),
            SlotValue::Qty(value) => Ok(Self::Qty(value)),
            SlotValue::Notional(value) => Ok(Self::Notional(value)),
            SlotValue::Text(_) => Err(CtxError::UnsupportedText),
        }
    }
}

impl From<SlotValueWire> for SlotValue {
    fn from(wire: SlotValueWire) -> Self {
        match wire {
            SlotValueWire::Bool(value) => Self::Bool(value),
            SlotValueWire::Decimal(value) => Self::Decimal(value),
            SlotValueWire::Symbol(value) => Self::Symbol(value),
            SlotValueWire::Side(value) => Self::Side(value),
            SlotValueWire::Px(value) => Self::Px(value),
            SlotValueWire::Qty(value) => Self::Qty(value),
            SlotValueWire::Notional(value) => Self::Notional(value),
        }
    }
}

/// Serializable field snapshot a policy DAG node consumes as its
/// typed input.
///
/// Build via [`PolicyCtxBuilder`] - usually right at the entry of a
/// policy DAG, snapshotting whatever fields the rule references off
/// the adopter's typed [`Context`] impl. Every downstream predicate
/// node receives a clone of the same `PolicyCtx`.
///
/// Keys are flattened to `"entity.name"` strings so the snapshot
/// round-trips through JSON (serde_json rejects map keys that aren't
/// strings). The flattening is private; lookups go through the
/// [`Context`] impl which takes the two parts separately.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct PolicyCtx {
    fields: BTreeMap<String, SlotValueWire>,
}

impl PolicyCtx {
    /// Start a new builder.
    pub fn builder() -> PolicyCtxBuilder {
        PolicyCtxBuilder::default()
    }

    /// Empty snapshot. Predicates that read fields will fail at
    /// evaluation time with [`crate::policy::EvalError::MissingField`].
    pub fn empty() -> Self {
        Self::default()
    }
}

impl Context for PolicyCtx {
    fn lookup(&self, entity: &str, name: &str) -> Option<SlotValue> {
        self.fields
            .get(&join_key(entity, name))
            .cloned()
            .map(SlotValue::from)
    }
}

/// Builder for [`PolicyCtx`]. Adopters write
/// `PolicyCtx::builder().field(entity, name, slot_value)?.build()`
/// or wire it through their own `Context::lookup`.
#[derive(Debug, Default)]
pub struct PolicyCtxBuilder {
    fields: BTreeMap<String, SlotValueWire>,
}

impl PolicyCtxBuilder {
    /// Capture one (entity, name) -> value pair. Returns an error
    /// only when the value is the unsupported `Text` variant.
    pub fn field(mut self, entity: &str, name: &str, value: SlotValue) -> Result<Self, CtxError> {
        let wire = SlotValueWire::try_from(value)?;
        self.fields.insert(join_key(entity, name), wire);
        Ok(self)
    }

    /// Snapshot every field listed in `fields` from the adopter's
    /// existing [`Context`] impl. Skips any field the context
    /// doesn't expose; returns an error on the first `Text`-valued
    /// field.
    pub fn snapshot_fields<C, I, S>(mut self, ctx: &C, fields: I) -> Result<Self, CtxError>
    where
        C: Context,
        I: IntoIterator<Item = (S, S)>,
        S: AsRef<str>,
    {
        for (entity, name) in fields {
            if let Some(value) = ctx.lookup(entity.as_ref(), name.as_ref()) {
                let wire = SlotValueWire::try_from(value)?;
                self.fields
                    .insert(join_key(entity.as_ref(), name.as_ref()), wire);
            }
        }
        Ok(self)
    }

    pub fn build(self) -> PolicyCtx {
        PolicyCtx {
            fields: self.fields,
        }
    }
}

fn join_key(entity: &str, name: &str) -> String {
    format!("{entity}.{name}")
}

/// Pass-through `DagCodec` impl so [`PolicyCtx`] can flow as a typed
/// input to apalis-workflow nodes. Delegates to whatever codec the
/// chosen backend uses (e.g. `JsonCodec<Value>` for `JsonStorage`).
impl<B, Err> DagCodec<B> for PolicyCtx
where
    B: BackendExt,
    B::Codec: Codec<Self, Compact = B::Compact, Error = Err>,
{
    type Error = Err;

    fn encode(self) -> Result<B::Compact, Self::Error> {
        B::Codec::encode(&self)
    }

    fn decode(response: &B::Compact) -> Result<Self, Self::Error> {
        B::Codec::decode(response)
    }
}

/// Pass-through `DagCodec` for the wire-side verdict tag. The verdict
/// task in [`crate::policy::backends::dag`] emits this so downstream
/// gating / routing nodes can act on the verdict.
impl<B, Err> DagCodec<B> for crate::policy::DecisionTag
where
    B: BackendExt,
    B::Codec: Codec<Self, Compact = B::Compact, Error = Err>,
{
    type Error = Err;

    fn encode(self) -> Result<B::Compact, Self::Error> {
        B::Codec::encode(&self)
    }

    fn decode(response: &B::Compact) -> Result<Self, Self::Error> {
        B::Codec::decode(response)
    }
}

/// Lets a `task_fn(...)` closure return `DecisionTag` directly. Apalis's
/// `IntoResponse` is the bridge between a closure's return type and a
/// `Service::Response`; the framework's primitives (`bool`, `i32`,
/// ...) ship with built-in impls, but adopter-defined types like
/// `DecisionTag` need an explicit one.
impl IntoResponse for crate::policy::DecisionTag {
    type Output = Self;

    fn into_response(self) -> Result<Self, BoxDynError> {
        Ok(self)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rust_decimal::Decimal;

    fn d(value: i64) -> Decimal {
        Decimal::from(value)
    }

    #[test]
    fn round_trips_through_json() {
        let ctx = PolicyCtx::builder()
            .field("order", "qty", SlotValue::Qty(Qty::new(d(7))))
            .unwrap()
            .field("order", "side", SlotValue::Side(Side::Buy))
            .unwrap()
            .build();

        let json = serde_json::to_string(&ctx).unwrap();
        let restored: PolicyCtx = serde_json::from_str(&json).unwrap();
        assert_eq!(ctx, restored);

        let qty = restored.lookup("order", "qty").expect("qty present");
        assert_eq!(qty, SlotValue::Qty(Qty::new(d(7))));
    }

    #[test]
    fn rejects_text_fields_at_snapshot_time() {
        let result = PolicyCtx::builder().field("info", "label", SlotValue::Text("hello"));
        assert!(matches!(result, Err(CtxError::UnsupportedText)));
    }

    #[test]
    fn lookup_returns_none_for_unsnapshotted_field() {
        let ctx = PolicyCtx::default();
        assert_eq!(ctx.lookup("order", "qty"), None);
    }

    /// `snapshot_fields` reads from any existing `Context` impl. Lets
    /// adopters keep their typed runtime context and just project the
    /// fields they need into the wire-shape.
    #[test]
    fn snapshot_fields_pulls_from_existing_context() {
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

        let snap = PolicyCtx::builder()
            .snapshot_fields(
                &OrderCtx {
                    qty: Qty::new(d(42)),
                },
                [("order", "qty"), ("order", "missing")],
            )
            .unwrap()
            .build();

        assert_eq!(
            snap.lookup("order", "qty"),
            Some(SlotValue::Qty(Qty::new(d(42))))
        );
        assert_eq!(snap.lookup("order", "missing"), None);
    }
}
