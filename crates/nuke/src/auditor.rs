//! First-class audit emission for reactor invocations.
//!
//! Borrowed from barter-rs's [`Auditor`] / [`AuditTick`] split:
//! the framework hands every reactor a place to record what it did
//! and why, in a typed shape adopters control.
//!
//! [`Auditor`] is the trait adopters implement (typically backed by
//! a [`crate::Tx`] to a metrics emitter, log line, file writer, or
//! cqrs/es store). [`AuditTick`] is the typed envelope - one tick
//! per reactor invocation, carrying a snapshot of the reactor's
//! relevant state plus the contextual event that triggered the tick.

use std::time::SystemTime;

/// One audit emission. Generic over the typed `Snapshot` (whatever
/// the adopter wants to capture: a position, a strategy state, a
/// risk envelope) and `Context` (the trigger - usually the event
/// the reactor just processed, or a derived summary of it).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AuditTick<Snapshot, Context> {
    /// When the tick was created, wall-clock.
    pub at: SystemTime,
    /// Monotonically-increasing sequence number for this auditor
    /// session. The auditor decides the meaning (per-reactor-instance,
    /// per-process, ...).
    pub sequence: u64,
    /// Reactor-state snapshot at the moment of the tick.
    pub snapshot: Snapshot,
    /// What triggered the tick.
    pub context: Context,
}

/// Receive [`AuditTick`]s. Implementors decide where they go - a
/// metrics exporter, an append-only log, a file, an event store -
/// and how failure is handled (drop, panic, retry).
pub trait Auditor<Snapshot, Context>: Send + Sync {
    /// Errors the auditor can surface. Implementor-defined.
    type Error: std::error::Error + Send + Sync + 'static;

    /// Emit one tick.
    fn emit(&self, tick: AuditTick<Snapshot, Context>) -> Result<(), Self::Error>;
}

/// No-op auditor: drops every tick, never errors. Useful as a
/// default placeholder when an adopter doesn't need an audit trail
/// (or hasn't wired one yet).
#[derive(Debug, Default, Clone, Copy)]
pub struct DropAuditor;

impl<Snapshot, Context> Auditor<Snapshot, Context> for DropAuditor {
    type Error = std::convert::Infallible;
    fn emit(&self, _tick: AuditTick<Snapshot, Context>) -> Result<(), Self::Error> {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    /// Test auditor that records every tick.
    struct RecordingAuditor<S: Clone, C: Clone> {
        ticks: Mutex<Vec<AuditTick<S, C>>>,
    }

    impl<S: Clone, C: Clone> RecordingAuditor<S, C> {
        fn new() -> Self {
            Self {
                ticks: Mutex::new(Vec::new()),
            }
        }
        fn snapshot(&self) -> Vec<AuditTick<S, C>> {
            self.ticks.lock().unwrap().clone()
        }
    }

    impl<S: Clone + Send + Sync, C: Clone + Send + Sync> Auditor<S, C> for RecordingAuditor<S, C> {
        type Error = std::convert::Infallible;
        fn emit(&self, tick: AuditTick<S, C>) -> Result<(), Self::Error> {
            self.ticks.lock().unwrap().push(tick);
            Ok(())
        }
    }

    #[test]
    fn auditor_records_ticks_with_snapshot_and_context() {
        let auditor: RecordingAuditor<&str, u32> = RecordingAuditor::new();
        auditor
            .emit(AuditTick {
                at: SystemTime::now(),
                sequence: 1,
                snapshot: "state:open",
                context: 42,
            })
            .unwrap();

        let ticks = auditor.snapshot();
        assert_eq!(ticks.len(), 1);
        assert_eq!(ticks[0].snapshot, "state:open");
        assert_eq!(ticks[0].context, 42);
        assert_eq!(ticks[0].sequence, 1);
    }

    #[test]
    fn drop_auditor_silently_succeeds() {
        let auditor = DropAuditor;
        let _ = auditor.emit(AuditTick::<(), ()> {
            at: SystemTime::now(),
            sequence: 0,
            snapshot: (),
            context: (),
        });
    }
}
