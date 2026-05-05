//! Generic, transport-agnostic walker over a type-level [`DepList`].
//!
//! A reactor's deps are a compile-time linked list of [`Subject`]
//! types (`Cons<A, Cons<B, Nil>>`). Before the run loop can deliver
//! events, every dep needs to be "opened" against some transport: a
//! ws connection wants one `subscribe` call per dep, a polling
//! client wants one fetcher task per dep, an in-process bus wants one
//! channel hookup per dep, and so on.
//!
//! The walking is the same in every case; only the per-dep step
//! differs. This module owns the walking and exposes two extension
//! points adapters fill in:
//!
//! - [`Transport`] - one impl per transport. Names the per-target-list
//!   accumulator built up while walking, via the [`Transport::Out`]
//!   GAT (so e.g. an address-keyed dispatcher can stay typed by the
//!   reactor's `L`).
//! - [`Wire<D>`] - one impl per (transport, dep family). Defines the
//!   per-dep step: open the subscription / register the decoder /
//!   spawn the poller / etc., mutating the accumulator.
//!
//! [`Subscribe::open_all`] is the entry point. It is implemented for
//! every [`DepList`] (blanket impls for [`Nil`] and [`Cons<H, Tail>`]
//! live below), so a caller writes:
//!
//! ```ignore
//! let acc = <R::Deps as Subscribe<R::Deps, MyTransport>>::open_all(
//!     &transport,
//! ).await?;
//! ```
//!
//! and gets back the transport's accumulator with one entry per dep.
//!
//! # Type-level shape
//!
//! [`Subscribe`] takes two list parameters because the *chain being
//! walked* shrinks at each recursion step while the *target list* the
//! accumulator is keyed by stays fixed:
//!
//! - `Self` is the chain remaining to walk. `Cons<H, Tail>` recurses
//!   on `Tail`, and `Nil` is the base case.
//! - `L` is the reactor's full original [`DepList`]. Every step's
//!   [`Wire<H>::wire`] call uses `L` to invoke
//!   `<L as HasDep<H>>::inject`, which is what places the decoded
//!   `(Id, Event)` into the right slot of the reactor's typed event
//!   union. `L` does not change during recursion.
//!
//! # Extending
//!
//! Adopters who add a new transport implement [`Transport`] once for
//! the transport type and [`Wire<D>`] for each dep family they
//! support. They never write recursion themselves; the blanket impls
//! in this module cover any [`DepList`] for that transport.

use std::future::Future;
use std::pin::Pin;

use crate::has_subject::HasDep;
use crate::subject::Subject;
use crate::subscribed::{Cons, DepList, Nil};

/// Boxed future returned by trait methods in this module. Kept as a
/// type alias to avoid `clippy::type_complexity` noise at every
/// signature - the boxing is unavoidable while [`Subscribe`] and
/// [`Wire`] are object-safe.
type BoxedFuture<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;

/// One transport against which a [`DepList`] can be opened.
///
/// A transport is a long-lived handle (a ws connection, an HTTP
/// client, an in-process bus, ...) that knows how to create
/// per-dep wirings. Implementors only need to declare the
/// accumulator type via [`Transport::Out`]; per-dep behavior lives in
/// [`Wire<D>`] impls on the same transport.
///
/// # The [`Transport::Out`] GAT
///
/// [`Transport::Out<L>`] is the value the walker hands back when it
/// finishes [`Subscribe::open_all`]. It is generic in the reactor's
/// target dep list `L` so per-list typing (e.g. decoders that emit
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
///
/// Implementations are typically blanket over an adapter-side trait:
///
/// ```ignore
/// impl<H: MyAdapterDep> Wire<H> for MyTransport
/// where
///     /* whatever bounds H needs */
/// {
///     fn wire<'a, L>(&'a self, out: &'a mut Dispatcher<L>)
///         -> BoxedFuture<'a, nuke::Result<()>>
///     where L: DepList + HasDep<H> + 'static,
///     {
///         /* open one subscription, register one decoder */
///     }
/// }
/// ```
///
/// The `L: HasDep<D>` bound on [`Wire::wire`] is what lets the body
/// call `<L as HasDep<D>>::inject(id, event)` to place a decoded
/// payload into the reactor's typed event union.
pub trait Wire<D: Subject>: Transport {
    /// Open this dep against `self`, mutating the accumulator with one
    /// entry for `D`.
    fn wire<'a, L>(&'a self, out: &'a mut Self::Out<L>) -> BoxedFuture<'a, crate::Result<()>>
    where
        L: DepList + HasDep<D> + 'static;
}

/// Type-level walker. Implemented for every [`DepList`] via the
/// blanket impls below.
///
/// Callers parameterize this with `(L, T)` where `L` is the reactor's
/// full target dep list and `T` is the [`Transport`] to open against;
/// the implementing chain is [`Self`] (which equals `L` at the entry
/// call but shrinks during recursion). [`Subscribe::open_all`]
/// returns the transport's accumulator [`T::Out<L>`].
///
/// See the module docs for the type-level shape and a worked example.
pub trait Subscribe<L: DepList, T: Transport> {
    /// Walk this chain against `transport`, calling [`Wire<H>::wire`]
    /// once for each dep `H` and returning the accumulated
    /// [`T::Out<L>`].
    fn open_all(transport: &T) -> BoxedFuture<'_, crate::Result<T::Out<L>>>;
}

/// Base case. Returns a fresh, empty [`T::Out<L>`].
impl<L: DepList + 'static, T: Transport> Subscribe<L, T> for Nil {
    fn open_all(_transport: &T) -> BoxedFuture<'_, crate::Result<T::Out<L>>> {
        Box::pin(async { Ok(T::Out::<L>::default()) })
    }
}

/// Recursive step. Walks `Tail` first to build the accumulator, then
/// runs [`Wire<H>::wire`] to add the entry for `H`. The `L` parameter
/// is held fixed across the recursion so every step's `Wire` call can
/// inject into the same target union.
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
