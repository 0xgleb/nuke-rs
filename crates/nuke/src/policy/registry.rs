//! Rule ID registry — every [`RuleId`](crate::policy::RuleId) used by a
//! policy is registered into a [`linkme`]-distributed slice at compile
//! time. CI then asserts:
//!
//! 1. **Uniqueness** — no two rules share an ID
//!    ([`assert_unique_ids`]).
//! 2. **Markdown coverage** — every registered rule has a matching
//!    file under `policies/` ([`assert_markdown_coverage`]) so
//!    compliance can read what a rule does without reading Rust.
//!
//! User code registers via [`register_rule!`]:
//!
//! ```ignore
//! use nuke::policy::{RuleId, register_rule};
//! register_rule!(RuleId::new("orders.max_size"), "orders/max_size.md");
//! ```
//!
//! The `policy!` macro (lands in task #20) will emit registrations
//! automatically; until then call `register_rule!` by hand.

use std::collections::HashSet;
use std::path::Path;

use linkme::distributed_slice;

use crate::policy::decision::RuleId;

/// One entry in the global rule registry.
#[derive(Debug, Clone, Copy)]
pub struct RuleEntry {
    pub id: RuleId,
    /// Path to the rule's markdown documentation, relative to the
    /// project's `policies/` directory.
    pub markdown_path: &'static str,
}

/// Distributed slice that every [`register_rule!`] invocation appends
/// to. Inspectable at runtime by tests / CI.
#[distributed_slice]
pub static RULES: [RuleEntry];

/// Snapshot of every registered rule. Returns a fresh `Vec` so callers
/// can sort/filter without affecting the global slice.
pub fn registered_rules() -> Vec<RuleEntry> {
    RULES.iter().copied().collect()
}

/// Errors a registry consistency check can produce.
#[derive(Debug, thiserror::Error)]
pub enum RegistryError {
    #[error("duplicate rule id: {0}")]
    Duplicate(RuleId),
    #[error("missing markdown for rule {id}: expected {path}")]
    MissingMarkdown { id: RuleId, path: String },
}

/// Assert that every registered [`RuleId`] is unique.
///
/// # Errors
///
/// Returns [`RegistryError::Duplicate`] on the first duplicate ID seen.
pub fn assert_unique_ids() -> Result<(), RegistryError> {
    let mut seen: HashSet<RuleId> = HashSet::new();
    for entry in RULES {
        if !seen.insert(entry.id) {
            return Err(RegistryError::Duplicate(entry.id));
        }
    }
    Ok(())
}

/// Assert that every registered rule has a markdown file at
/// `<policies_dir>/<entry.markdown_path>`.
///
/// # Errors
///
/// Returns [`RegistryError::MissingMarkdown`] on the first missing file.
pub fn assert_markdown_coverage(policies_dir: &Path) -> Result<(), RegistryError> {
    for entry in RULES {
        let path = policies_dir.join(entry.markdown_path);
        if !path.exists() {
            return Err(RegistryError::MissingMarkdown {
                id: entry.id,
                path: path.display().to_string(),
            });
        }
    }
    Ok(())
}

/// Register a [`RuleId`] + its markdown path. Place once per rule at
/// the module level; linkme handles the rest.
///
/// ```ignore
/// register_rule!(RuleId::new("orders.max_size"), "orders/max_size.md");
/// ```
#[macro_export]
macro_rules! register_rule {
    ($id:expr, $markdown:expr) => {
        const _: () = {
            #[$crate::reexports::linkme::distributed_slice($crate::policy::registry::RULES)]
            #[linkme(crate = $crate::reexports::linkme)]
            static _ENTRY: $crate::policy::registry::RuleEntry =
                $crate::policy::registry::RuleEntry {
                    id: $id,
                    markdown_path: $markdown,
                };
        };
    };
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Registry tests can't easily assert specific contents (other tests
    /// in the binary may register rules), but they CAN assert that
    /// uniqueness holds for whatever IS registered.
    #[test]
    fn no_duplicate_ids_among_registered_rules() {
        assert_unique_ids().expect("rule ID registry contains duplicates");
    }

    #[test]
    fn registered_rules_snapshot_is_consistent_size() {
        let snapshot = registered_rules();
        // Identity: snapshot count matches the live slice.
        assert_eq!(snapshot.len(), RULES.len());
    }
}
