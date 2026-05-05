//! JSON Schema backend - emits the minimal Draft 2020-12 schema
//! describing the context fields a `RuleNode` reads. Any system feeding
//! the rule must conform to this schema; CI can diff the schema across
//! revisions.

use std::collections::{BTreeMap, BTreeSet};

use serde_json::{Map, Value, json};

use crate::policy::ast::{BinOpExpr, CmpExpr, FieldRef, InnerExpr, LitValue, RuleNode};

/// Emit the JSON Schema for the context fields a rule reads.
///
/// Field types are inferred from any matching `LitValue` literals seen
/// alongside the field; if no literal disambiguates, the schema falls
/// back to an unconstrained type.
pub fn render(rule: &RuleNode) -> Value {
    let mut fields: BTreeMap<String, FieldShape> = BTreeMap::new();
    walk_rule(rule, &mut fields);

    let mut entities: BTreeMap<String, BTreeMap<String, Value>> = BTreeMap::new();
    let mut required: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
    for (path, shape) in fields {
        let (entity, name) = path.split_once('.').expect("walker emits dotted paths");
        entities
            .entry(entity.to_owned())
            .or_default()
            .insert(name.to_owned(), shape.to_schema());
        required
            .entry(entity.to_owned())
            .or_default()
            .insert(name.to_owned());
    }

    let mut top_props: Map<String, Value> = Map::new();
    for (entity, properties) in entities {
        let mut entity_obj = Map::new();
        entity_obj.insert("type".into(), Value::String("object".into()));
        let mut props_map: Map<String, Value> = Map::new();
        for (name, schema) in properties {
            props_map.insert(name, schema);
        }
        entity_obj.insert("properties".into(), Value::Object(props_map));
        if let Some(req) = required.get(&entity) {
            entity_obj.insert(
                "required".into(),
                Value::Array(req.iter().map(|name| Value::String(name.clone())).collect()),
            );
        }
        top_props.insert(entity, Value::Object(entity_obj));
    }

    json!({
        "$schema": "https://json-schema.org/draft/2020-12/schema",
        "type": "object",
        "properties": top_props,
        "required": top_props.keys().cloned().collect::<Vec<_>>(),
    })
}

#[derive(Default, Debug, Clone, Copy)]
struct FieldShape {
    inferred: Option<InferredType>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum InferredType {
    Bool,
    /// Numeric: any of `Decimal`, `Px`, `Qty`, `Notional`. We don't
    /// distinguish in the schema layer for v0.
    Number,
    String,
}

impl FieldShape {
    fn merge(&mut self, other: InferredType) {
        match self.inferred {
            None => self.inferred = Some(other),
            Some(prev) if prev == other => {}
            Some(_) => self.inferred = None, // conflict -> fall back to unconstrained
        }
    }

    fn to_schema(self) -> Value {
        match self.inferred {
            Some(InferredType::Bool) => json!({ "type": "boolean" }),
            Some(InferredType::Number) => json!({ "type": "string", "format": "decimal" }),
            Some(InferredType::String) => json!({ "type": "string" }),
            None => Value::Object(Map::new()),
        }
    }
}

fn walk_rule(rule: &RuleNode, fields: &mut BTreeMap<String, FieldShape>) {
    match rule {
        RuleNode::Given { conditions, then } => {
            for condition in conditions {
                walk_expr(condition, fields);
            }
            walk_rule(then, fields);
        }
        RuleNode::RejectIf { condition, .. } | RuleNode::EscalateIf { condition, .. } => {
            walk_expr(condition, fields);
        }
        RuleNode::All(rules) | RuleNode::Any(rules) => {
            for sub in rules {
                walk_rule(sub, fields);
            }
        }
        RuleNode::Bind { expr, then, .. } => {
            walk_expr(expr, fields);
            walk_rule(then, fields);
        }
        // `Run` references slots already bound earlier in the rule,
        // so it adds no new field requirements to the input schema.
        RuleNode::Run(_) => {}
    }
}

fn walk_expr(expr: &InnerExpr, fields: &mut BTreeMap<String, FieldShape>) {
    match expr {
        InnerExpr::Lit(_) | InnerExpr::Slot(_) => {}
        InnerExpr::Field(FieldRef { entity, name }) => {
            fields.entry(format!("{entity}.{name}")).or_default();
        }
        InnerExpr::Cmp(CmpExpr { lhs, rhs, .. }) | InnerExpr::BinOp(BinOpExpr { lhs, rhs, .. }) => {
            // Co-infer when one side is a literal and the other a field.
            infer_pair(lhs, rhs, fields);
            infer_pair(rhs, lhs, fields);
            walk_expr(lhs, fields);
            walk_expr(rhs, fields);
        }
        InnerExpr::Not(inner) => walk_expr(inner, fields),
        InnerExpr::And(parts) | InnerExpr::Or(parts) => {
            for part in parts {
                walk_expr(part, fields);
            }
        }
    }
}

fn infer_pair(
    field_side: &InnerExpr,
    other_side: &InnerExpr,
    fields: &mut BTreeMap<String, FieldShape>,
) {
    let InnerExpr::Field(FieldRef { entity, name }) = field_side else {
        return;
    };
    let InnerExpr::Lit(value) = other_side else {
        return;
    };
    let inferred = match value {
        LitValue::Bool(_) => InferredType::Bool,
        LitValue::Decimal(_) | LitValue::Px(_) | LitValue::Qty(_) | LitValue::Notional(_) => {
            InferredType::Number
        }
        LitValue::Symbol(_) | LitValue::Side(_) | LitValue::Text(_) => InferredType::String,
    };
    fields
        .entry(format!("{entity}.{name}"))
        .or_default()
        .merge(inferred);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::Qty;
    use crate::policy::ast::{Expr, QtyT, field, gt};
    use crate::policy::decision::RuleId;
    use crate::policy::reason::Reason;

    #[test]
    fn infers_required_fields_with_numeric_type_from_lit() {
        let condition = gt(
            field::<QtyT>("order", "qty"),
            Expr::<QtyT>::lit(Qty::new(rust_decimal::Decimal::from(100))),
        )
        .into_inner();
        let rule = RuleNode::RejectIf {
            rule: RuleId::new("orders.too_large"),
            condition,
            reason: Reason::literal("nope"),
        };
        let schema = render(&rule);
        let order = schema
            .pointer("/properties/order")
            .expect("order entity emitted");
        assert_eq!(order["type"], "object");
        let qty_schema = order.pointer("/properties/qty").expect("qty field emitted");
        assert_eq!(qty_schema["type"], "string");
        assert_eq!(qty_schema["format"], "decimal");
        let required = order["required"].as_array().expect("required array");
        assert!(required.iter().any(|v| v == "qty"));
    }
}
