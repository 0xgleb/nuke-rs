//! Generic, transport-agnostic walker over a reactor's [`DepList`].
//!
//! A reactor declares its deps via the [`deps!`](crate::deps) macro;
//! at run-loop startup the framework needs to "open" each dep against
//! some transport (a ws connection, a polling client, an in-process
//! channel, ...) and accumulate per-dep wiring (decoders, handles,
//! filters, ...) into a transport-defined output.
//!
//! That recursion is what lives here. It is not adapter-specific. Each
//! transport supplies:
//!
//! - a single [`Transport`] impl that names the per-target-list
//!   accumulator type via the [`Transport::Out`] GAT, and
//! - one [`Wire`] impl per dep family it knows how to open (e.g. the
//!   EVM adapter impls `Wire<H>` for any `H: EvmSubject` against
//!   `EvmWsSource`).
//!
//! [`Subscribe`] then walks any [`DepList`] for that transport,
//! producing the transport's accumulator. Adopters do not write
//! recursive type-level code; they only describe one step.

use std::future::Future;
use std::pin::Pin;

use crate::has_subject::HasDep;
use crate::subject::Subject;
use crate::subscribed::{Cons, DepList, Nil};

/// Boxed future returned by trait methods in this module. Kept as a
/// type alias to avoid `clippy::type_complexity` noise at every call
/// site - the trait machinery here is monomorphized per-(L, T) and
/// the boxing is unavoidable while these traits are object-safe.
type BoxedFuture<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;

/// A transport against which a reactor's deps can be opened.
///
/// The associated [`Transport::Out`] is per-target-DepList: a single
/// transport may build a different accumulator depending on the
/// reactor's dep list (e.g. EVM's `Dispatcher<L>` is keyed by the
/// reactor's specific `L` so its decoders inject into the correct
/// event union).
pub trait Transport: Send + Sync {
    /// Per-list accumulator the walker builds up. `L: 'static` because
    /// the accumulator typically holds boxed closures that close over
    /// `L`-typed handles.
    type Out<L: DepList + 'static>: Default + Send + 'static;
}

/// Per-dep wiring step. Implemented by a transport for each dep
/// family it can open (typically via a blanket bounded by an
/// adapter-side trait, e.g. `impl<H: EvmSubject> Wire<H> for
/// EvmWsSource`).
pub trait Wire<D: Subject>: Transport {
    /// Open this dep against `self`, mutating the per-list
    /// accumulator.
    fn wire<'a, L>(&'a self, out: &'a mut Self::Out<L>) -> BoxedFuture<'a, crate::Result<()>>
    where
        L: DepList + HasDep<D> + 'static;
}

/// Type-level walker that opens each dep in `Self` (a [`DepList`])
/// against a transport `T`, returning `T::Out<L>` (the reactor's
/// per-list accumulator).
///
/// `Self` is the chain currently being walked; `L` is the reactor's
/// full target dep list. The recursion preserves `L` so each step's
/// `Wire<H>` can call `<L as HasDep<H>>::inject` with the correct
/// target union.
///
/// Blanket impls for [`Cons`] and [`Nil`] live in this crate;
/// adapters only implement [`Transport`] + [`Wire`].
pub trait Subscribe<L: DepList, T: Transport> {
    fn open_all(transport: &T) -> BoxedFuture<'_, crate::Result<T::Out<L>>>;
}

impl<L: DepList, T: Transport> Subscribe<L, T> for Nil {
    fn open_all(_transport: &T) -> BoxedFuture<'_, crate::Result<T::Out<L>>> {
        Box::pin(async { Ok(T::Out::<L>::default()) })
    }
}

impl<H, Tail, L, T> Subscribe<L, T> for Cons<H, Tail>
where
    H: Subject,
    L: DepList + HasDep<H> + 'static,
    T: Transport + Wire<H>,
    Tail: Subscribe<L, T>,
{
    fn open_all(transport: &T) -> BoxedFuture<'_, crate::Result<T::Out<L>>> {
        Box::pin(async move {
            let mut out = Tail::open_all(transport).await?;
            <T as Wire<H>>::wire::<L>(transport, &mut out).await?;
            Ok(out)
        })
    }
}
