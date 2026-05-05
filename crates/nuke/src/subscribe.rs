//! Generic, transport-agnostic walker over a [`DepList`]. Adopters
//! who add a new transport implement [`Transport`] once for the
//! transport type and [`Wire<D>`] for each dep family they support;
//! the recursion (blanket impls for [`Cons`] and [`Nil`]) lives here.
//!
//! Type-level shape: [`Subscribe<L, T>`] takes two list parameters
//! because the chain being walked shrinks at each recursion step
//! while the target list the accumulator is keyed by stays fixed.
//! `L` is the reactor's full original [`DepList`]; every step's
//! [`Wire<H>::wire`] call uses `L` to invoke `<L as HasDep<H>>::inject`,
//! which places the decoded `(Id, Event)` into the right slot of the
//! reactor's typed event union.

use std::future::Future;
use std::pin::Pin;

use event_sorcery::{Cons, DepList, HasDep, Nil};

use crate::subject::Subject;

/// Boxed future returned by trait methods in this module. Kept as a
/// type alias to avoid `clippy::type_complexity` noise at every
/// signature - the boxing is unavoidable while [`Subscribe`] and
/// [`Wire`] are object-safe.
type BoxedFuture<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;

/// One transport against which a [`DepList`] can be opened.
///
/// A transport is a long-lived handle (a ws connection, an HTTP client,
/// an in-process bus, ...) that knows how to create per-dep wirings.
/// Implementors only need to declare the accumulator type via
/// [`Transport::Out`]; per-dep behavior lives in [`Wire<D>`] impls on
/// the same transport.
///
/// [`Transport::Out<L>`] is generic in the reactor's target dep list
/// `L` so per-list typing (e.g. decoders that emit
/// `<L as DepList>::Event`) survives end-to-end. The `L: 'static`
/// bound exists because accumulators routinely hold boxed closures
/// that close over `L`-typed values.
pub trait Transport: Send + Sync {
    /// Per-target-DepList accumulator the walker builds up.
    type Out<L: DepList + 'static>: Default + Send + 'static;
}

/// Per-dep wiring step on a [`Transport`].
///
/// One impl per (transport, dep family). The body of [`Wire::wire`]
/// is the only adapter-specific code in the open-all path: it reads
/// `D`'s associated types / consts (via [`Subject`] and any extension
/// trait the dep family agreed on) and mutates the transport's
/// accumulator with one entry for `D`.
pub trait Wire<D: Subject>: Transport {
    /// Open this dep against `self`, mutating the accumulator with
    /// one entry for `D`.
    fn wire<'a, L>(&'a self, out: &'a mut Self::Out<L>) -> BoxedFuture<'a, crate::Result<()>>
    where
        L: DepList + HasDep<D> + 'static;
}

/// Type-level walker. Implemented for every [`DepList`] via the
/// blanket impls below. Callers parameterize this with `(L, T)` where
/// `L` is the reactor's full target dep list and `T` is the
/// [`Transport`] to open against; the implementing chain is `Self`
/// (which equals `L` at the entry call but shrinks during recursion).
pub trait Subscribe<L: DepList, T: Transport> {
    /// Walk this chain against `transport`, calling [`Wire<H>::wire`]
    /// once for each dep `H` and returning the accumulated
    /// [`Transport::Out`].
    fn open_all(transport: &T) -> BoxedFuture<'_, crate::Result<T::Out<L>>>;
}

impl<L: DepList + 'static, T: Transport> Subscribe<L, T> for Nil {
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
