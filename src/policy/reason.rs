//! [`Reason`] — typed format-string AST and [`Bindings`] — captured
//! evaluation values.
//!
//! Stored separately from a flat `String` so every backend (markdown
//! digest, CBOR wire format, semantic diff, telemetry) can render or
//! inspect the structured parts. The template itself is a sequence of
//! literal chunks and named slots; the slot values come from
//! `Bindings`, which a runtime evaluator populates as it walks the rule.

use rust_decimal::Decimal;

use crate::domain::{Notional, Px, Qty, Side, Symbol};

/// A structured rejection / escalation reason.
///
/// Either a flat literal (`"order rejected"`) or a templated message
/// with named slots (`"order qty {requested} exceeds limit {limit}"`).
/// Backends choose how to render — markdown can interpolate, CBOR
/// preserves both template and slot values, semantic diff compares
/// templates structurally.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Reason {
    pub template: ReasonTemplate,
    pub slots: Vec<Slot>,
}

impl Reason {
    /// Build a reason with no named slots.
    pub fn literal(text: &'static str) -> Self {
        Self {
            template: ReasonTemplate::Literal(text),
            slots: Vec::new(),
        }
    }

    /// Build a reason from a template and the slots that fill it.
    pub fn templated(template: &'static [ReasonChunk], slots: Vec<Slot>) -> Self {
        Self {
            template: ReasonTemplate::Chunks(template),
            slots,
        }
    }

    /// Render the reason as a flat string. Backends needing structure
    /// should walk `template` and `slots` directly instead.
    pub fn render(&self) -> String {
        match &self.template {
            ReasonTemplate::Literal(text) => (*text).to_owned(),
            ReasonTemplate::Chunks(chunks) => chunks
                .iter()
                .map(|chunk| match chunk {
                    ReasonChunk::Literal(text) => (*text).to_owned(),
                    ReasonChunk::Slot(name) => self
                        .slots
                        .iter()
                        .find(|slot| slot.name == *name)
                        .map_or_else(|| format!("{{{name}}}"), |slot| format!("{}", slot.value)),
                })
                .collect(),
        }
    }
}

/// The structured template a [`Reason`] expands. Keeps backends able to
/// distinguish "literal text the rule author wrote" from "values
/// captured at evaluation time."
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ReasonTemplate {
    /// No interpolation — the message is exactly the literal text.
    Literal(&'static str),
    /// Sequence of literal chunks and named slots.
    ///
    /// Stored as `&'static` so the template lives for the program's
    /// lifetime and CI can validate uniqueness across all rules without
    /// reflective allocation. Macro-generated callers will produce a
    /// `&'static` array via `const`.
    Chunks(&'static [ReasonChunk]),
}

/// One element of a templated reason.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReasonChunk {
    Literal(&'static str),
    Slot(SlotName),
}

/// A named slot in a [`Reason`] template, paired with its evaluated
/// value at runtime.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Slot {
    pub name: SlotName,
    pub value: SlotValue,
}

/// Stable name for a slot. `&'static str` so backends can compare and
/// index by reference equality.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct SlotName(pub &'static str);

impl std::fmt::Display for SlotName {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(self.0)
    }
}

/// Typed value that can fill a [`Slot`].
///
/// One variant per supported domain type — extending the universe of
/// slot values is intentionally a deliberate change here, not free-form
/// `String` interpolation. New variants must be added when new domain
/// primitives appear.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SlotValue {
    Bool(bool),
    Decimal(Decimal),
    Symbol(Symbol),
    Side(Side),
    Px(Px),
    Qty(Qty),
    Notional(Notional),
    Text(&'static str),
}

impl std::fmt::Display for SlotValue {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Bool(value) => write!(formatter, "{value}"),
            Self::Decimal(value) => write!(formatter, "{value}"),
            Self::Symbol(value) => write!(formatter, "{value}"),
            Self::Side(value) => write!(formatter, "{value}"),
            Self::Px(value) => write!(formatter, "{value}"),
            Self::Qty(value) => write!(formatter, "{value}"),
            Self::Notional(value) => write!(formatter, "{value}"),
            Self::Text(value) => formatter.write_str(value),
        }
    }
}

/// Values captured during rule evaluation that produced the verdict.
///
/// Keyed by [`SlotName`] so a [`Reason`] template's slots can be
/// resolved against the actual values, and so backends (markdown,
/// telemetry, CBOR audit) can render a complete record of *why* a rule
/// reached its decision — not just what it returned.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Bindings {
    entries: Vec<(SlotName, SlotValue)>,
}

impl Bindings {
    pub const fn empty() -> Self {
        Self {
            entries: Vec::new(),
        }
    }

    pub fn from_pairs<I>(pairs: I) -> Self
    where
        I: IntoIterator<Item = (SlotName, SlotValue)>,
    {
        Self {
            entries: pairs.into_iter().collect(),
        }
    }

    pub fn capture(&mut self, name: SlotName, value: SlotValue) {
        self.entries.push((name, value));
    }

    pub fn get(&self, name: SlotName) -> Option<&SlotValue> {
        self.entries
            .iter()
            .find(|(slot_name, _)| *slot_name == name)
            .map(|(_, value)| value)
    }

    pub fn iter(&self) -> impl Iterator<Item = &(SlotName, SlotValue)> {
        self.entries.iter()
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
    fn literal_reason_renders_verbatim() {
        let reason = Reason::literal("order rejected");
        assert_eq!(reason.render(), "order rejected");
    }

    #[test]
    fn templated_reason_renders_with_slots() {
        const TEMPLATE: &[ReasonChunk] = &[
            ReasonChunk::Literal("order qty "),
            ReasonChunk::Slot(SlotName("requested")),
            ReasonChunk::Literal(" exceeds limit "),
            ReasonChunk::Slot(SlotName("limit")),
        ];
        let reason = Reason::templated(
            TEMPLATE,
            vec![
                Slot {
                    name: SlotName("requested"),
                    value: SlotValue::Decimal(d(150)),
                },
                Slot {
                    name: SlotName("limit"),
                    value: SlotValue::Decimal(d(100)),
                },
            ],
        );
        assert_eq!(reason.render(), "order qty 150 exceeds limit 100");
    }

    #[test]
    fn templated_reason_falls_back_when_slot_missing() {
        const TEMPLATE: &[ReasonChunk] = &[
            ReasonChunk::Literal("missing: "),
            ReasonChunk::Slot(SlotName("nope")),
        ];
        let reason = Reason::templated(TEMPLATE, vec![]);
        assert_eq!(reason.render(), "missing: {nope}");
    }

    #[test]
    fn bindings_capture_and_lookup() {
        let mut bindings = Bindings::empty();
        bindings.capture(SlotName("size"), SlotValue::Decimal(d(42)));
        assert_eq!(
            bindings.get(SlotName("size")),
            Some(&SlotValue::Decimal(d(42)))
        );
        assert_eq!(bindings.get(SlotName("missing")), None);
    }

    #[test]
    fn bindings_preserve_insertion_order() {
        let bindings = Bindings::from_pairs(vec![
            (SlotName("a"), SlotValue::Bool(true)),
            (SlotName("b"), SlotValue::Bool(false)),
        ]);
        let names: Vec<_> = bindings.iter().map(|(name, _)| *name).collect();
        assert_eq!(names, vec![SlotName("a"), SlotName("b")]);
    }
}
