//! CBOR wire format + schema hash. Used for post-trade audits - every
//! deployed rule is encoded once, hashed, and the hash is recorded
//! alongside the verdict; later, the audit replays the exact rule
//! that ran. Also enables hot-reload by shipping just the encoded
//! bytes + hash without recompiling.

use serde::Serialize;
use sha2::{Digest, Sha256};

/// Compact wire format: CBOR-encoded `RuleNode` plus its SHA-256 hash.
#[derive(Debug, Clone)]
pub struct WireRule {
    pub bytes: Vec<u8>,
    pub schema_hash: [u8; 32],
}

impl WireRule {
    /// Borrow the schema hash as a hex string for logs / audit records.
    pub fn schema_hash_hex(&self) -> String {
        self.schema_hash
            .iter()
            .fold(String::with_capacity(64), |mut acc, byte| {
                use std::fmt::Write;
                write!(&mut acc, "{byte:02x}").ok();
                acc
            })
    }
}

/// Encode a [`RuleNode`] (or any `Serialize` AST node) to the wire format.
pub fn encode<T: Serialize>(rule: &T) -> Result<WireRule, ciborium::ser::Error<std::io::Error>> {
    let mut bytes = Vec::with_capacity(256);
    ciborium::into_writer(rule, &mut bytes)?;

    let mut hasher = Sha256::new();
    hasher.update(&bytes);
    let digest: [u8; 32] = hasher.finalize().into();

    Ok(WireRule {
        bytes,
        schema_hash: digest,
    })
}

/// Encode just to bytes - useful when you only need the bytes (e.g.
/// for a custom hash or alternative storage).
pub fn encode_bytes<T: Serialize>(
    rule: &T,
) -> Result<Vec<u8>, ciborium::ser::Error<std::io::Error>> {
    let mut bytes = Vec::with_capacity(256);
    ciborium::into_writer(rule, &mut bytes)?;
    Ok(bytes)
}

/// Compute just the schema hash without retaining the bytes.
pub fn schema_hash<T: Serialize>(
    rule: &T,
) -> Result<[u8; 32], ciborium::ser::Error<std::io::Error>> {
    Ok(encode(rule)?.schema_hash)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::Qty;
    use crate::policy::ast::{Expr, QtyT, RuleNode, field, gt};
    use crate::policy::decision::RuleId;
    use crate::policy::reason::Reason;

    fn sample_rule() -> RuleNode {
        let condition = gt(
            field::<QtyT>("order", "qty"),
            Expr::<QtyT>::lit(Qty::new(rust_decimal::Decimal::from(100))),
        )
        .into_inner();
        RuleNode::RejectIf {
            rule: RuleId::new("orders.too_large"),
            condition,
            reason: Reason::literal("nope"),
        }
    }

    #[test]
    fn encode_round_trips_to_some_bytes() {
        let wire = encode(&sample_rule()).unwrap();
        assert!(!wire.bytes.is_empty());
        assert_eq!(wire.schema_hash.len(), 32);
    }

    #[test]
    fn schema_hash_is_stable_across_runs() {
        let a = schema_hash(&sample_rule()).unwrap();
        let b = schema_hash(&sample_rule()).unwrap();
        assert_eq!(a, b, "same rule must hash to same digest");
    }

    #[test]
    fn schema_hash_changes_when_rule_changes() {
        let original = sample_rule();
        let modified: RuleNode = match original.clone() {
            RuleNode::RejectIf {
                rule, condition, ..
            } => RuleNode::RejectIf {
                rule,
                condition,
                reason: Reason::literal("different reason"),
            },
            other => panic!("unexpected: {other:?}"),
        };
        let a = schema_hash(&original).unwrap();
        let b = schema_hash(&modified).unwrap();
        assert_ne!(a, b, "different rules must hash differently");
    }

    #[test]
    fn schema_hash_hex_is_64_hex_chars() {
        let wire = encode(&sample_rule()).unwrap();
        let hex = wire.schema_hash_hex();
        assert_eq!(hex.len(), 64);
        assert!(hex.chars().all(|c| c.is_ascii_hexdigit()));
    }
}
