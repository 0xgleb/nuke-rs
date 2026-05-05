//! The [`Reactor`] trait — what reacts to events from a list of subjects.
//!
//! The event type is *computed* from `Subscribed::Subjects`, not declared
//! by hand. Implement `react` with the `.on(...).on(...).exhaustive()`
//! chain — forgetting a subject is a compile error.

use async_trait::async_trait;
use std::sync::Arc;

use crate::subscribed::{SubjectList, Subscribed};

/// Event reactor with exhaustive compile-time-checked handling.
///
/// ```ignore
/// subjects!(ArbBot, [UniV2WethUsdc, SushiV2WethUsdc]);
///
/// #[async_trait]
/// impl Reactor for ArbBot {
///     type Error = ArbError;
///
///     async fn react(
///         &self,
///         event: <Self::Subjects as SubjectList>::Event,
///     ) -> Result<(), Self::Error> {
///         event
///             .on(|id, sync| async move { self.on_univ2(id, sync).await })
///             .on(|id, sync| async move { self.on_sushi(id, sync).await })
///             .exhaustive()
///             .await
///     }
/// }
/// ```
#[async_trait]
pub trait Reactor: Subscribed + Send + Sync {
    type Error: std::error::Error + Send + Sync + 'static;

    async fn react(&self, event: <Self::Subjects as SubjectList>::Event)
    -> Result<(), Self::Error>;
}

#[async_trait]
impl<R> Reactor for Arc<R>
where
    R: Reactor,
{
    type Error = R::Error;

    async fn react(
        &self,
        event: <Self::Subjects as SubjectList>::Event,
    ) -> Result<(), Self::Error> {
        R::react(self, event).await
    }
}
