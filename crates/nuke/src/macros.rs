//! The [`subjects!`] declarative macro and its `register_subjects!`
//! helper. Mirrors event-sorcery's `deps!` / `register_entities!`.
//!
//! Two forms, distinguished by syntax:
//!
//! - **Type-position** `subjects![A, B, C]` expands to a nested
//!   `Cons<A, Cons<B, Cons<C, Nil>>>` type.
//! - **Statement-position** `subjects!(Reactor, [A, B, C])` generates
//!   `impl Subscribed for Reactor` and `HasSubject<X>` impls for each
//!   subject — written once, no duplication.

/// Build a type-level subject list from subject types.
#[macro_export]
macro_rules! subjects {
    // Statement-position: generate Subscribed + HasSubject impls.
    ($reactor:ty, [$($subject:ty),+ $(,)?]) => {
        impl $crate::Subscribed for $reactor {
            type Subjects = $crate::subjects![$($subject),+];
        }
        $crate::register_subjects!($($subject),+);
    };

    // Type-position: expand to Cons chain.
    () => { $crate::Nil };
    ($head:ty $(, $tail:ty)* $(,)?) => {
        $crate::Cons<$head, $crate::subjects![$($tail),*]>
    };
}

/// Generate [`HasSubject`](crate::HasSubject) impls for each subject in
/// a list.
///
/// Low-level building block used by the statement-position form of
/// [`subjects!`]. Prefer [`subjects!`] directly.
#[macro_export]
macro_rules! register_subjects {
    // Single subject: blanket `HasSubject<S> for Cons<S, Nil>` covers it.
    ($single:ty) => {};

    // Multiple subjects: walk the list and generate one impl per subject.
    ($($subject:ty),+ $(,)?) => {
        $crate::register_subjects!(@impls [$($subject),+] [$($subject),+] []);
    };

    // All subjects processed.
    (@impls [] [$($all:ty),+] [$($done:ty),*]) => {};

    // Generate HasSubject for the current subject, then recurse.
    (@impls [$current:ty $(, $rest:ty)*] [$($all:ty),+] [$($done:ty),*]) => {
        impl $crate::HasSubject<$current> for $crate::subjects![$($all),+] {
            fn inject(
                id: <$current as $crate::Subject>::Id,
                event: <$current as $crate::Subject>::Event,
            ) -> <Self as $crate::SubjectList>::Event {
                $crate::register_subjects!(@wrap [$($done),*] $crate::OneOf::Here((id, event)))
            }
        }
        $crate::register_subjects!(@impls [$($rest),*] [$($all),+] [$($done,)* $current]);
    };

    // No wrapping needed (subject is at head).
    (@wrap [] $expr:expr) => { $expr };

    // Wrap in OneOf::There once per subject preceding `current`.
    (@wrap [$_head:ty $(, $rest:ty)*] $expr:expr) => {
        $crate::OneOf::There($crate::register_subjects!(@wrap [$($rest),*] $expr))
    };
}
