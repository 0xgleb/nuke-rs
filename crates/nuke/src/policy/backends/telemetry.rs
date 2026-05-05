//! Coverage / drift telemetry - counts how often each leaf in a
//! `RuleNode` fires in production.
//!
//! Surfaces dead rules (zero hits over a window -> a candidate for
//! removal) and regime changes (a rule that used to fire 100x/day
//! suddenly going quiet means market behaviour shifted, not that the
//! code is broken). Backends emit metrics; this module just keeps the
//! per-`RuleId` counters and exposes a snapshot.

use std::collections::BTreeMap;
use std::sync::Mutex;

use crate::policy::decision::{Decision, RuleId};

/// Singleton in-memory counter registry. For production deployments,
/// wire `record_decision` to a metrics exporter (Prometheus, OTel)
/// instead of inspecting [`snapshot`].
static COUNTERS: Mutex<Counters> = Mutex::new(Counters::new());

#[derive(Debug, Default)]
struct Counters {
    by_rule: BTreeMap<RuleId, RuleCounters>,
}

impl Counters {
    const fn new() -> Self {
        Self {
            by_rule: BTreeMap::new(),
        }
    }
}

/// Fire counts per outcome for a single rule.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct RuleCounters {
    pub denies: u64,
    pub escalations: u64,
}

/// Record a `Decision` against the global registry.
pub fn record_decision(decision: &Decision) {
    let mut guard = COUNTERS.lock().expect("telemetry counters poisoned");
    match decision {
        Decision::Allow => {}
        Decision::Deny { rule, .. } => {
            guard.by_rule.entry(*rule).or_default().denies += 1;
        }
        Decision::Escalate { rule, .. } => {
            guard.by_rule.entry(*rule).or_default().escalations += 1;
        }
    }
}

/// Snapshot of the current counter state. Returns a fresh map so the
/// caller can sort/filter without holding the global lock.
pub fn snapshot() -> BTreeMap<RuleId, RuleCounters> {
    COUNTERS
        .lock()
        .expect("telemetry counters poisoned")
        .by_rule
        .clone()
}

/// Reset every counter - primarily for tests.
pub fn reset() {
    COUNTERS
        .lock()
        .expect("telemetry counters poisoned")
        .by_rule
        .clear();
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::policy::reason::{Bindings, Reason};

    fn deny(id: &'static str) -> Decision {
        Decision::Deny {
            rule: RuleId::new(id),
            reason: Reason::literal("test"),
            bindings: Bindings::empty(),
        }
    }

    fn escalate(id: &'static str) -> Decision {
        Decision::Escalate {
            rule: RuleId::new(id),
            reason: Reason::literal("test"),
            to: crate::policy::EscalationTarget::new("desk"),
            bindings: Bindings::empty(),
        }
    }

    #[test]
    fn counters_increment_per_outcome() {
        // Use a unique rule id so other tests can't perturb us.
        reset();
        record_decision(&deny("telemetry.test.alpha"));
        record_decision(&deny("telemetry.test.alpha"));
        record_decision(&escalate("telemetry.test.alpha"));
        record_decision(&Decision::Allow); // no counter change
        let snap = snapshot();
        let counts = snap
            .get(&RuleId::new("telemetry.test.alpha"))
            .copied()
            .expect("counters present");
        assert_eq!(counts.denies, 2);
        assert_eq!(counts.escalations, 1);
        reset();
    }
}
