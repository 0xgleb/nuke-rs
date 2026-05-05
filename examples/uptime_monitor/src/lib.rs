//! Polling-Source example for the nuke framework.
//!
//! Two service-health "pingers" implemented as [`nuke::ExtQuery`]
//! impls poll their target on a tokio interval (lifted to a
//! [`nuke::ExtStream`] by [`nuke::Polling`]); the merged events are
//! fed to a [`nuke::Reactor`] via [`nuke::pump_dep_streams`]. The
//! reactor remembers the last status per service and enqueues an
//! [`AlertJob`] when one transitions.
//!
//! The example deliberately avoids real HTTP - the [`HealthQuery`]
//! reads from a deterministic sequence so the inline test can assert
//! exactly which alerts fire. Running the binary with no arguments
//! exercises the same wiring against a live tokio interval and a
//! looping fixture.

use std::collections::{HashMap, VecDeque};
use std::sync::Arc;

use async_trait::async_trait;
use nuke::prelude::*;
use nuke::{DepStream, ExtQuery, Job, Label, Polling, inject_ext_stream, pump_dep_streams};
use serde::{Deserialize, Serialize};
use tokio::sync::Mutex;

/// Two distinct dep markers - one per monitored service. Sharing the
/// same payload (`HealthStatus`) is fine; the discriminated dep union
/// is what tells the reactor which service produced an event.
pub struct PrimaryApi;
pub struct SecondaryApi;

/// Stable id for one monitored service. The reactor uses it to key
/// its last-seen-status table.
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct ServiceId(pub String);

/// Health verdict emitted on each poll.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum HealthStatus {
    Healthy,
    Degraded,
    Down,
}

impl Dep for PrimaryApi {
    type Id = ServiceId;
    type Event = HealthStatus;
}

impl Dep for SecondaryApi {
    type Id = ServiceId;
    type Event = HealthStatus;
}

/// Mock [`ExtQuery`] reading from a prebuilt sequence. Demonstrates
/// the polling shape without committing the example to a specific
/// HTTP client; an adopter swaps this for a real `reqwest::Client`
/// (or similar) without touching the wiring downstream.
#[derive(Clone)]
pub struct HealthQuery {
    sequence: Arc<Mutex<VecDeque<HealthStatus>>>,
}

impl HealthQuery {
    pub fn new<I: IntoIterator<Item = HealthStatus>>(samples: I) -> Self {
        Self {
            sequence: Arc::new(Mutex::new(samples.into_iter().collect())),
        }
    }
}

#[derive(Debug, thiserror::Error)]
pub enum HealthError {
    #[error("health query exhausted - no more samples")]
    Exhausted,
}

impl ExtQuery for HealthQuery {
    type Request = ();
    type Response = HealthStatus;
    type Error = HealthError;

    async fn query(&self, _: ()) -> Result<HealthStatus, HealthError> {
        self.sequence
            .lock()
            .await
            .pop_front()
            .ok_or(HealthError::Exhausted)
    }
}

/// Job emitted when the reactor detects a service status transition.
/// Idempotent: re-running `perform` only re-logs the transition; no
/// external side-effects in this example.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct AlertJob {
    pub service: ServiceId,
    pub from: HealthStatus,
    pub to: HealthStatus,
}

impl Job<MonitorCtx> for AlertJob {
    type Error = std::convert::Infallible;

    fn label(&self) -> Label {
        Label::new(format!("alert/{}", self.service.0))
    }

    async fn perform(&self, ctx: &MonitorCtx) -> Result<(), Self::Error> {
        ctx.alerts.lock().await.push(self.clone());
        ::tracing::info!(
            service = %self.service.0,
            from = ?self.from,
            to = ?self.to,
            "service status transition"
        );
        Ok(())
    }
}

/// Shared context for [`AlertJob::perform`]. Holds the alert sink the
/// inline test (and a real adopter's notification client) reads from.
#[derive(Default)]
pub struct MonitorCtx {
    pub alerts: Mutex<Vec<AlertJob>>,
}

/// Reactor: per-service last-seen state in a mutex; emits an
/// [`AlertJob`] on transition.
pub struct MonitorBot {
    last_seen: Mutex<HashMap<ServiceId, HealthStatus>>,
}

impl MonitorBot {
    pub fn new() -> Self {
        Self {
            last_seen: Mutex::new(HashMap::new()),
        }
    }

    async fn note(&self, id: ServiceId, status: HealthStatus) -> Vec<AlertJob> {
        let mut state = self.last_seen.lock().await;
        match state.insert(id.clone(), status) {
            Some(previous) if previous != status => vec![AlertJob {
                service: id,
                from: previous,
                to: status,
            }],
            _ => Vec::new(),
        }
    }
}

impl Default for MonitorBot {
    fn default() -> Self {
        Self::new()
    }
}

deps!(MonitorBot, [PrimaryApi, SecondaryApi]);

#[async_trait]
impl Reactor for MonitorBot {
    type Job = AlertJob;
    type Ctx = MonitorCtx;

    async fn react(&self, event: <Self::Deps as DepList>::Event) -> Vec<AlertJob> {
        event
            .on(|id, status| async move { self.note(id, status).await })
            .on(|id, status| async move { self.note(id, status).await })
            .exhaustive()
            .await
    }
}

/// Wire one [`HealthQuery`] to a tokio interval and lift the
/// resulting [`ExtStream`] into the reactor's typed dep union.
///
/// The `D` parameter selects which arm of the union the events land
/// in (so the reactor's `.on(...)` handlers stay disambiguated).
pub fn poll_service<D>(
    id: ServiceId,
    query: HealthQuery,
    schedule: impl futures_util::Stream<Item = ()> + Send + Sync + 'static,
) -> DepStream<<MonitorBot as Dependent>::Deps>
where
    D: Dep<Id = ServiceId, Event = HealthStatus>,
    <MonitorBot as Dependent>::Deps: HasDep<D>,
{
    inject_ext_stream::<D, <MonitorBot as Dependent>::Deps, _>(
        id,
        Polling::new(query, (), schedule),
    )
}

/// Run the monitor against the supplied per-service streams. Returns
/// once the apalis worker shuts down (which only happens when both
/// upstreams complete).
pub async fn run(
    streams: Vec<DepStream<<MonitorBot as Dependent>::Deps>>,
    ctx: Arc<MonitorCtx>,
) -> nuke::Result<()> {
    pump_dep_streams(streams, Arc::new(MonitorBot::new()), ctx).await
}

#[cfg(test)]
mod tests {
    use super::*;
    use futures_util::stream;

    /// Drive the reactor directly against deterministic per-dep
    /// sequences and verify the alert ledger. This isolates the
    /// `inject_ext_stream` + `select_all` + `react` path; the apalis
    /// worker is not spun up because it would block on an empty queue
    /// once the streams complete.
    #[tokio::test]
    async fn detects_status_transitions_per_service() {
        let primary = poll_service::<PrimaryApi>(
            ServiceId("primary".into()),
            HealthQuery::new([
                HealthStatus::Healthy,
                HealthStatus::Degraded,
                HealthStatus::Down,
            ]),
            stream::iter(std::iter::repeat_n((), 3)),
        );
        let secondary = poll_service::<SecondaryApi>(
            ServiceId("secondary".into()),
            HealthQuery::new([HealthStatus::Healthy, HealthStatus::Healthy]),
            stream::iter(std::iter::repeat_n((), 2)),
        );

        let bot = MonitorBot::new();
        let merged = futures_util::stream::select_all(vec![primary, secondary]);
        let events: Vec<_> = futures_util::StreamExt::collect(merged).await;
        assert_eq!(events.len(), 5);

        let mut alerts = Vec::new();
        for event_result in events {
            let event = event_result.expect("query infallible against a finite sample sequence");
            alerts.extend(bot.react(event).await);
        }

        // primary: Healthy -> Degraded -> Down (two transitions).
        // secondary: Healthy -> Healthy (no transition).
        assert_eq!(
            alerts,
            vec![
                AlertJob {
                    service: ServiceId("primary".into()),
                    from: HealthStatus::Healthy,
                    to: HealthStatus::Degraded,
                },
                AlertJob {
                    service: ServiceId("primary".into()),
                    from: HealthStatus::Degraded,
                    to: HealthStatus::Down,
                },
            ]
        );
    }

    /// First sample for a previously-unseen service is *not* a
    /// transition (we have nothing to compare against).
    #[tokio::test]
    async fn first_sample_does_not_emit_an_alert() {
        let bot = MonitorBot::new();
        let alerts = bot
            .note(ServiceId("only".into()), HealthStatus::Healthy)
            .await;
        assert!(alerts.is_empty());
    }
}
