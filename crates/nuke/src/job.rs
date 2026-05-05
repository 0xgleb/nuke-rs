//! [`Job<Ctx>`] - the unit of durable, retryable work.
//!
//! Reactors emit Jobs; apalis stores them, hands them to a Worker,
//! and applies retries via [`backon`]. Every external-service
//! interaction (POST / signed-tx / message-send / venue
//! `place_trade`) lives inside a Job's `perform` so that durability,
//! retries, and backoff come for free.
//!
//! The framework's run loop never invokes a `TradingVenue` method
//! directly - it always goes through a Job. The Job carries the
//! `Ctx` (a struct of injected services / handles) it needs, which
//! apalis hands in via the `Data<Arc<Ctx>>` extractor.

use std::fmt;
use std::sync::Arc;

use apalis::prelude::{BoxDynError, Data};
use backon::{ExponentialBuilder, Retryable};
use serde::Serialize;
use serde::de::DeserializeOwned;

/// A persistent, retryable unit of work.
///
/// Implementations are serializable structs that carry the data
/// needed to process a single occurrence. The `Ctx` type parameter
/// bundles all runtime dependencies (executor handles, CQRS
/// frameworks, `TradingVenue` impls, config, etc.) into one struct
/// injected via apalis `Data<Arc<Ctx>>`.
///
/// The framework provides the generic [`work`] handler that bridges
/// Job impls into apalis's function-handler API; users register
/// `work::<Ctx, MyJob>` with `WorkerBuilder::build`.
#[allow(async_fn_in_trait)]
pub trait Job<Ctx>: Serialize + DeserializeOwned + Send + 'static
where
    Ctx: Send + Sync + 'static,
{
    /// Error type returned by [`perform`](Job::perform).
    type Error: std::error::Error + Send + Sync + 'static;

    /// Human-readable label for structured logging.
    fn label(&self) -> Label;

    /// Process this job using the provided context.
    ///
    /// The returned future must be `Send` so apalis can run it on a
    /// multi-threaded executor; the framework's run loop is unconditionally
    /// `Send`-bound.
    fn perform(
        &self,
        ctx: &Ctx,
    ) -> impl std::future::Future<Output = Result<(), Self::Error>> + Send;
}

/// Human-readable identifier for an enqueued job. Used in structured
/// logging and as a stable key in policy [`crate::policy::ast::ActionSpec`]
/// references.
#[derive(Debug, Clone, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
pub struct Label(String);

impl Label {
    pub fn new(label: impl Into<String>) -> Self {
        Self(label.into())
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for Label {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

/// Generic apalis handler that bridges [`Job`] implementations with
/// apalis's function-based worker API. Wraps [`Job::perform`] in an
/// exponential-backoff retry policy.
///
/// Register with apalis via:
///
/// ```ignore
/// WorkerBuilder::new(name)
///     .backend(storage)
///     .data(ctx)
///     .build(work::<MyCtx, MyJob>)
/// ```
pub async fn work<Ctx, J>(job: J, ctx: Data<Arc<Ctx>>)
where
    Ctx: Send + Sync + 'static,
    J: Job<Ctx> + Sync,
{
    const MAX_RETRIES: usize = 3;
    let label = job.label();
    tracing::debug!(%label, max_retries = MAX_RETRIES, "starting job");

    let result = (|| job.perform(&ctx))
        .retry(ExponentialBuilder::default().with_max_times(MAX_RETRIES))
        .notify(|error, duration| {
            tracing::warn!(%label, %error, ?duration, "retrying job after transient failure");
        })
        .await;

    if let Err(error) = result {
        tracing::error!(%label, %error, "job failed after retries");
    }
}

/// Box wrapper used by adapters that bridge stream items into job
/// errors.
pub type DynJobError = BoxDynError;
