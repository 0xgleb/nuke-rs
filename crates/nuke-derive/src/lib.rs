//! Procedural macros for nuke-rs.
//!
//! Currently only `#[derive(EvmSubject)]`, which generates a `Subject` impl
//! from `#[nuke(event = ABI::Variant, address = "0x...")]`.

use proc_macro::TokenStream;

mod domain;
mod evm_subject;

/// Derive `nuke::Subject` for a unit struct that names an on-chain log
/// source. Usage:
///
/// ```ignore
/// #[derive(EvmSubject)]
/// #[nuke(event = UniswapV2Pair::Sync, address = "B4e16d0168e52d35CaCD2c6185b44281Ec28C9Dc")]
/// pub struct UniV2WethUsdc;
/// ```
#[proc_macro_derive(EvmSubject, attributes(nuke))]
pub fn derive_evm_subject(input: TokenStream) -> TokenStream {
    evm_subject::expand(input.into())
        .unwrap_or_else(syn::Error::into_compile_error)
        .into()
}

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
