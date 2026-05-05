//! The [`deps!`] declarative macro and its `register_deps!`
//! helper.
//!
//! Two forms, distinguished by syntax:
//!
//! - **Type-position** `deps![A, B, C]` expands to a nested
//!   `Cons<A, Cons<B, Cons<C, Nil>>>` type.
//! - **Statement-position** `deps!(Reactor, [A, B, C])` generates
//!   `impl Dependent for Reactor` and `HasDep<X>` impls for each
//!   dep - written once, no duplication.
//!
//! Naming mirrors event-sorcery's `deps!` so reactor declarations read
//! the same way regardless of whether the dep is an external stream or
//! an internal aggregate.

/// Build a type-level dep list from dep types.
#[macro_export]
macro_rules! deps {
    // Statement-position: generate Dependent + HasDep impls.
    ($reactor:ty, [$($dep:ty),+ $(,)?]) => {
        impl $crate::Dependent for $reactor {
            type Deps = $crate::deps![$($dep),+];
        }
        $crate::register_deps!($($dep),+);
    };

    // Type-position: expand to Cons chain.
    () => { $crate::Nil };
    ($head:ty $(, $tail:ty)* $(,)?) => {
        $crate::Cons<$head, $crate::deps![$($tail),*]>
    };
}

/// Generate [`HasDep`](crate::HasDep) impls for each dep in a list.
///
/// Low-level building block used by the statement-position form of
/// [`deps!`]. Prefer [`deps!`] directly.
#[macro_export]
macro_rules! register_deps {
    // Single dep: blanket `HasDep<D> for Cons<D, Nil>` covers it.
    ($single:ty) => {};

    // Multiple deps: walk the list and generate one impl per dep.
    ($($dep:ty),+ $(,)?) => {
        $crate::register_deps!(@impls [$($dep),+] [$($dep),+] []);
    };

    // All deps processed.
    (@impls [] [$($all:ty),+] [$($done:ty),*]) => {};

    // Generate HasDep for the current dep, then recurse.
    (@impls [$current:ty $(, $rest:ty)*] [$($all:ty),+] [$($done:ty),*]) => {
        impl $crate::HasDep<$current> for $crate::deps![$($all),+] {
            fn inject(
                id: <$current as $crate::Subject>::Id,
                event: <$current as $crate::Subject>::Event,
            ) -> <Self as $crate::DepList>::Event {
                $crate::register_deps!(@wrap [$($done),*] $crate::OneOf::Here((id, event)))
            }
        }
        $crate::register_deps!(@impls [$($rest),*] [$($all),+] [$($done,)* $current]);
    };

    // No wrapping needed (dep is at head).
    (@wrap [] $expr:expr) => { $expr };

    // Wrap in OneOf::There once per dep preceding `current`.
    (@wrap [$_head:ty $(, $rest:ty)*] $expr:expr) => {
        $crate::OneOf::There($crate::register_deps!(@wrap [$($rest),*] $expr))
    };
}
