//! Expansion for `#[derive(EvmSubject)]`.
//!
//! Reads `#[nuke(event = Path::To::Event, address = "hex")]` and emits:
//! - a newtype `Id` (`<Type>Id(alloy_primitives::Address)`) implementing
//!   `Debug`, `Display`, `Clone`, plus `From<Address>` and `Deref`,
//! - a `Subject` impl whose `Event` is the named alloy event type, whose
//!   `subscription()` returns a logs filter for the configured address +
//!   the event's `SIGNATURE_HASH`, and whose `decode()` defers to the
//!   alloy event's `SolEvent::decode_log_data`.

use proc_macro2::TokenStream;
use quote::{format_ident, quote};
use syn::{Attribute, DeriveInput, LitStr, Path, parse2};

pub(crate) fn expand(input: TokenStream) -> syn::Result<TokenStream> {
    let derive: DeriveInput = parse2(input)?;
    let name = &derive.ident;

    let attrs = NukeAttrs::from_attrs(&derive.attrs)?;
    let event_path = attrs.event.ok_or_else(|| {
        syn::Error::new_spanned(
            &derive.ident,
            "missing #[nuke(event = ABI::Variant)] attribute",
        )
    })?;
    let address_lit = attrs.address.ok_or_else(|| {
        syn::Error::new_spanned(
            &derive.ident,
            "missing #[nuke(address = \"0x...\")] attribute",
        )
    })?;

    let address_hex = address_lit.value();
    let address_normalized = address_hex.trim_start_matches("0x");
    if address_normalized.len() != 40 || !address_normalized.chars().all(|c| c.is_ascii_hexdigit())
    {
        return Err(syn::Error::new_spanned(
            &address_lit,
            "address must be a 40-char hex string (with or without 0x prefix)",
        ));
    }
    let address_with_prefix = format!("0x{address_normalized}");

    let id_ident = format_ident!("{}Id", name);
    let name_str = name.to_string();

    Ok(quote! {
        #[doc = concat!("Strongly-typed identifier for the `", stringify!(#name), "` subject.")]
        #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
        pub struct #id_ident(pub ::nuke::reexports::alloy_primitives::Address);

        impl ::core::fmt::Display for #id_ident {
            fn fmt(&self, formatter: &mut ::core::fmt::Formatter<'_>) -> ::core::fmt::Result {
                ::core::fmt::Display::fmt(&self.0, formatter)
            }
        }

        impl ::core::convert::From<::nuke::reexports::alloy_primitives::Address> for #id_ident {
            fn from(address: ::nuke::reexports::alloy_primitives::Address) -> Self {
                Self(address)
            }
        }

        impl ::core::ops::Deref for #id_ident {
            type Target = ::nuke::reexports::alloy_primitives::Address;
            fn deref(&self) -> &Self::Target { &self.0 }
        }

        impl ::nuke::Subject for #name {
            type Id = #id_ident;
            type Event = #event_path;

            const NAME: &'static str = #name_str;
            const SCHEMA_VERSION: u64 = 1;

            fn address() -> ::nuke::reexports::alloy_primitives::Address {
                ::nuke::reexports::alloy_primitives::address!(#address_with_prefix)
            }

            fn subscription() -> ::nuke::evm::SubscriptionSpec {
                use ::nuke::reexports::alloy_sol_types::SolEvent;
                ::nuke::evm::SubscriptionSpec::logs_for(
                    Self::address(),
                    <#event_path as SolEvent>::SIGNATURE_HASH,
                )
            }

            fn decode(log: &::nuke::evm::RawLog) -> ::core::result::Result<
                Self::Event,
                ::nuke::evm::DecodeError,
            > {
                use ::nuke::reexports::alloy_sol_types::SolEvent;
                <#event_path as SolEvent>::decode_log_data(log.data())
                    .map_err(::nuke::evm::DecodeError::from)
            }
        }
    })
}

struct NukeAttrs {
    event: Option<Path>,
    address: Option<LitStr>,
}

impl NukeAttrs {
    fn from_attrs(attrs: &[Attribute]) -> syn::Result<Self> {
        let mut event: Option<Path> = None;
        let mut address: Option<LitStr> = None;

        for attr in attrs {
            if !attr.path().is_ident("nuke") {
                continue;
            }
            attr.parse_nested_meta(|meta| {
                if meta.path.is_ident("event") {
                    let value = meta.value()?;
                    let path: Path = value.parse()?;
                    event = Some(path);
                    return Ok(());
                }
                if meta.path.is_ident("address") {
                    let value = meta.value()?;
                    let lit: LitStr = value.parse()?;
                    address = Some(lit);
                    return Ok(());
                }
                Err(meta.error("unsupported #[nuke(...)] key; expected `event` or `address`"))
            })?;
        }

        Ok(Self { event, address })
    }
}
