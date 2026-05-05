//! Lifecycle hooks adopters implement to react to resilience events
//! the framework can't make a default decision about.
//!
//! Three hooks:
//!
//! - [`OnDisconnect`] - the source's transport dropped. The hook
//!   returns the [`DisconnectAction`] the run loop should take
//!   (reconnect with backoff / give up / pause and wait).
//! - [`OnTradingDisabled`] - operator-side flip (a circuit-breaker, a
//!   compliance hold). Reactor decides what to do with in-flight work.
//! - [`OnShutdown`] - graceful stop. Adopter cancels in-flight work,
//!   flushes audit, etc.
//!
//! All three are deliberately separate traits (no one giant
//! `Lifecycle` god-trait) so adopters opt in to only the events
//! they care about. Default-Allow / default-Reconnect implementations
//! ship for the no-effort case.

use std::time::Duration;

/// Action the run loop should take after a transport disconnect.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DisconnectAction {
    /// Reconnect after the given delay.
    Reconnect { backoff: Duration },
    /// Stop the run loop. The adopter has decided this disconnect is
    /// unrecoverable.
    Shutdown,
    /// Hold the run loop in a paused state until an external trigger
    /// (operator action, healthcheck signal, ...) resumes it. The
    /// adopter is responsible for the resume side-channel.
    Pause,
}

/// Hook invoked when an external transport disconnects.
pub trait OnDisconnect: Send + Sync {
    /// Decide what to do. `attempt` is the consecutive disconnect
    /// count since the last clean connect (1 for the first
    /// disconnect of a session) so adopters can ramp backoff.
    fn on_disconnect(&self, attempt: u32) -> DisconnectAction;
}

/// Default impl: exponential backoff capped at 30s, never gives up.
/// Adequate for "ws transport that flaps" but probably wrong for
/// "the venue is permanently gone" - adopters override when they
/// have richer signal.
#[derive(Debug, Default, Clone, Copy)]
pub struct DefaultDisconnect;

impl OnDisconnect for DefaultDisconnect {
    fn on_disconnect(&self, attempt: u32) -> DisconnectAction {
        // 1s, 2s, 4s, 8s, 16s, 30s, 30s, ...
        let backoff_secs = 1u64.checked_shl(attempt.saturating_sub(1)).unwrap_or(30);
        DisconnectAction::Reconnect {
            backoff: Duration::from_secs(backoff_secs.min(30)),
        }
    }
}

/// Hook invoked when an operator (or upstream signal) flips trading
/// off mid-session. The reactor decides what to do with in-flight
/// work and whether to keep listening to the bus or fully pause.
pub trait OnTradingDisabled: Send + Sync {
    /// Called once per "trading-off" event. Implementors typically
    /// cancel open orders, flush state, and either pause the reactor
    /// or let the surrounding orchestration tear it down.
    fn on_trading_disabled(&self);
}

/// Default impl: no-op. Adopters that do nothing on disable can
/// reuse this; anyone with cleanup work overrides.
#[derive(Debug, Default, Clone, Copy)]
pub struct DefaultTradingDisabled;

impl OnTradingDisabled for DefaultTradingDisabled {
    fn on_trading_disabled(&self) {}
}

/// Hook invoked when the run loop is asked to stop. Implementors
/// flush audit, cancel in-flight work, drain the apalis worker.
pub trait OnShutdown: Send + Sync {
    fn on_shutdown(&self);
}

/// Default impl: no-op.
#[derive(Debug, Default, Clone, Copy)]
pub struct DefaultShutdown;

impl OnShutdown for DefaultShutdown {
    fn on_shutdown(&self) {}
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_disconnect_ramps_backoff_and_caps_at_thirty_seconds() {
        let h = DefaultDisconnect;
        let secs = |a: u32| match h.on_disconnect(a) {
            DisconnectAction::Reconnect { backoff } => backoff.as_secs(),
            other => panic!("expected Reconnect, got {other:?}"),
        };
        assert_eq!(secs(1), 1);
        assert_eq!(secs(2), 2);
        assert_eq!(secs(3), 4);
        assert_eq!(secs(4), 8);
        assert_eq!(secs(5), 16);
        assert_eq!(secs(6), 30);
        assert_eq!(secs(20), 30, "deep backoffs cap at 30s");
    }

    #[test]
    fn default_trading_disabled_is_a_noop() {
        DefaultTradingDisabled.on_trading_disabled();
    }

    #[test]
    fn default_shutdown_is_a_noop() {
        DefaultShutdown.on_shutdown();
    }
}
