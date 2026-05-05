//! Run the uptime monitor against a couple of looping fixtures
//! polled on a tokio interval. Demonstrates the wiring; not intended
//! as a deployable health checker.

use std::sync::Arc;
use std::time::Duration;

use futures_util::StreamExt;
use tokio_stream::wrappers::IntervalStream;
use uptime_monitor::{
    HealthQuery, HealthStatus, MonitorCtx, PrimaryApi, SecondaryApi, ServiceId, poll_service, run,
};

fn schedule(period: Duration) -> impl futures_util::Stream<Item = ()> + Send + Sync + 'static {
    IntervalStream::new(tokio::time::interval(period)).map(|_| ())
}

#[tokio::main]
async fn main() -> nuke::Result<()> {
    tracing_subscriber::fmt::try_init().ok();

    let primary = poll_service::<PrimaryApi>(
        ServiceId("primary".into()),
        HealthQuery::new(
            [
                HealthStatus::Healthy,
                HealthStatus::Degraded,
                HealthStatus::Healthy,
            ]
            .repeat(2),
        ),
        schedule(Duration::from_millis(250)),
    );
    let secondary = poll_service::<SecondaryApi>(
        ServiceId("secondary".into()),
        HealthQuery::new([HealthStatus::Healthy, HealthStatus::Down].repeat(2)),
        schedule(Duration::from_millis(400)),
    );

    let ctx = Arc::new(MonitorCtx::default());
    run(vec![primary, secondary], ctx).await
}
