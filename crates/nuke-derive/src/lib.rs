//! Proc-macros exported by this crate.
//!
//! - [`Domain`] - generates typed accessors and a `read_field` method
//!   on a domain entity struct so the eDSL can address its fields by
//!   name without losing static type checks.

use proc_macro::TokenStream;

mod domain;

/// Derive `nuke::policy` accessors for a domain entity. Usage:
///
/// ```ignore
/// #[derive(Domain)]
/// pub struct Order {
///     pub qty: Qty,
///     pub side: Side,
///     pub price: Px,
/// }
///
/// // Generates:
/// //   pub mod order { pub fn qty() -> Expr<QtyT> { ... } pub fn side() -> Expr<SideT> { ... } ... }
/// //   impl Order { pub fn read_field(&self, name: &str) -> Option<SlotValue> { ... } }
/// ```
#[proc_macro_derive(Domain)]
pub fn derive_domain(input: TokenStream) -> TokenStream {
    domain::expand(input.into())
        .unwrap_or_else(syn::Error::into_compile_error)
        .into()
}
