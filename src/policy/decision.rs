//! [`Decision`] — the total verdict algebra returned by every rule
//! evaluation.
//!
//! Three variants and only three: `Allow`, `Deny`, `Escalate`. Verdict
//! types that don't appear here can't be returned by a rule, which keeps
//! every backend's match exhaustive.

use crate::policy::reason::{Bindings, Reason};

/// What a rule decides about an input.
///
/// `Deny` and `Escalate` carry the full evaluation context (which rule,
/// the structured `Reason`, the captured `Bindings`) so a rejection is
/// self-explanatory to a trader and reproducible to an auditor.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Decision {
    /// The rule is satisfied; the input may proceed.
    Allow,
    /// The rule rejected the input.
    Deny {
        rule: RuleId,
        reason: Reason,
        bindings: Bindings,
    },
    /// The rule wants a human (or another system) to decide.
    Escalate {
        rule: RuleId,
        to: EscalationTarget,
        reason: Reason,
        bindings: Bindings,
    },
}

impl Decision {
    /// True iff this decision permits the input to proceed.
    pub fn is_allow(&self) -> bool {
        matches!(self, Self::Allow)
    }

    /// True iff this decision blocks the input outright.
    pub fn is_deny(&self) -> bool {
        matches!(self, Self::Deny { .. })
    }

    /// True iff this decision defers to an escalation handler.
    pub fn is_escalate(&self) -> bool {
        matches!(self, Self::Escalate { .. })
    }
}

/// Stable identifier for a rule. Interned `&'static str` so equality is
/// pointer-cheap and IDs never invalidate.
///
/// IDs are registered at startup via the rule registry (lands with the
/// `policy!` macro epic). Creating a `RuleId` directly bypasses that
/// registry — only do it from generated code.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct RuleId(&'static str);

impl RuleId {
    /// Construct a `RuleId` from a `&'static str`. Intended for
    /// macro-generated callers; hand-written usage should pre-register
    /// through the rule registry once it lands.
    pub const fn new(name: &'static str) -> Self {
        Self(name)
    }

    pub const fn as_str(&self) -> &'static str {
        self.0
    }
}

impl std::fmt::Display for RuleId {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(self.0)
    }
}

/// Where an `Escalate` decision should be routed (e.g. `"compliance"`,
/// `"risk-desk"`, `"manual-review"`). Backends decide what to do with
/// the target — the rule itself just names it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct EscalationTarget(&'static str);

impl EscalationTarget {
    pub const fn new(name: &'static str) -> Self {
        Self(name)
    }

    pub const fn as_str(&self) -> &'static str {
        self.0
    }
}

impl std::fmt::Display for EscalationTarget {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(self.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::policy::reason::{Bindings, Reason};

    #[test]
    fn allow_classifiers_work() {
        let allow = Decision::Allow;
        assert!(allow.is_allow());
        assert!(!allow.is_deny());
        assert!(!allow.is_escalate());
    }

    #[test]
    fn deny_classifiers_work() {
        let deny = Decision::Deny {
            rule: RuleId::new("test.deny"),
            reason: Reason::literal("blocked"),
            bindings: Bindings::empty(),
        };
        assert!(!deny.is_allow());
        assert!(deny.is_deny());
        assert!(!deny.is_escalate());
    }

    #[test]
    fn escalate_classifiers_work() {
        let escalate = Decision::Escalate {
            rule: RuleId::new("test.escalate"),
            to: EscalationTarget::new("compliance"),
            reason: Reason::literal("review"),
            bindings: Bindings::empty(),
        };
        assert!(!escalate.is_allow());
        assert!(!escalate.is_deny());
        assert!(escalate.is_escalate());
    }

    #[test]
    fn rule_id_round_trips_through_display() {
        let id = RuleId::new("orders.max_size");
        assert_eq!(id.as_str(), "orders.max_size");
        assert_eq!(format!("{id}"), "orders.max_size");
    }
}
