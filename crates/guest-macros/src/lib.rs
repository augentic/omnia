#![doc = include_str!("../README.md")]

//! Procedural attributes for Omnia guests.

mod capability;
mod from_row;
mod otel;
mod provider;
mod settings;

use proc_macro::TokenStream;
use quote::quote;
use syn::{DeriveInput, ItemFn, ItemTrait, meta, parse_macro_input};

/// Instruments a function using the `[wasi_otel::instrument]` function.
///
/// This macro can be used to automatically create spans for functions, making
/// it easier to add observability to your code.
#[proc_macro_attribute]
pub fn instrument(args: TokenStream, item: TokenStream) -> TokenStream {
    // macro's attributes
    let mut attrs = otel::Attributes::default();
    let arg_parser = meta::parser(|meta| attrs.parse(&meta));
    parse_macro_input!(args with arg_parser);

    let item_fn = parse_macro_input!(item as ItemFn);
    let body = otel::body(attrs, &item_fn);

    // Re-emit the function's own attributes, visibility, and signature so the
    // instrumented wrapper keeps its docs, `pub`, and any `#[cfg]`/`#[allow]`.
    let fn_attrs = &item_fn.attrs;
    let vis = &item_fn.vis;
    let signature = &item_fn.sig;

    let new_fn = quote! {
        #(#fn_attrs)*
        #vis #signature {
            #body
        }
    };

    TokenStream::from(new_fn)
}

/// Makes a trait a capability that `#[derive(Provider)]` can delegate to a field.
///
/// Re-exported as `omnia_sdk::capability`. Beside the trait it emits:
///
/// - a hidden `Has<Trait>` accessor trait, naming the type it delegates to
///   as `Inner`;
/// - a blanket impl of the trait for every `Has<Trait>`, forwarding each
///   method and associated type to `Inner`;
/// - `Has<Trait>` impls for `Arc<P>`, `&P`, and `Box<P>`.
///
/// A crate that re-exports the trait re-exports `Has<Trait>` beside it, so
/// `#[provider(path::Trait)]` finds `path::HasTrait`. A generic or defaulted
/// associated type is rejected.
#[proc_macro_attribute]
pub fn capability(_args: TokenStream, item: TokenStream) -> TokenStream {
    let item_trait = parse_macro_input!(item as ItemTrait);
    match capability::expand(&item_trait) {
        Ok(tokens) => TokenStream::from(tokens),
        Err(err) => TokenStream::from(err.into_compile_error()),
    }
}

/// Derives capability traits by delegating each one to the field that names it.
///
/// A field annotated `#[provider(..)]` serves the capabilities it lists:
///
/// - A bare name is an `omnia_sdk` capability, as `#[provider(Config, StateStore)]`.
/// - A path names any other `#[capability]` trait, as `#[provider(crate::Geocoder)]`.
///   Its `Has<Trait>` must be reachable at the same path.
///
/// The field's type is part of the provider's interface, so a `pub` provider
/// delegates only to `pub` field types.
#[proc_macro_derive(Provider, attributes(provider))]
pub fn derive_provider(item: TokenStream) -> TokenStream {
    let input = parse_macro_input!(item as DeriveInput);
    match provider::expand(&input) {
        Ok(tokens) => TokenStream::from(tokens),
        Err(err) => TokenStream::from(err.into_compile_error()),
    }
}

/// Derives a `KEYS` list and an `async fn load(config)` that reads every field
/// of a named-field struct from `omnia_sdk::Config` through
/// `omnia_sdk::ConfigExt::require`.
///
/// A field's key is its name upper-cased behind the struct's
/// `#[setting(prefix = "..")]`, or the literal in `#[setting(key = "..")]`.
#[proc_macro_derive(Settings, attributes(setting))]
pub fn derive_settings(item: TokenStream) -> TokenStream {
    let input = parse_macro_input!(item as DeriveInput);
    match settings::expand(&input) {
        Ok(tokens) => TokenStream::from(tokens),
        Err(err) => TokenStream::from(err.into_compile_error()),
    }
}

/// Derives `omnia_sdk::sql::FromRow` for a named-field struct, reading each
/// field from the column of the same name through `Row::get`.
#[proc_macro_derive(FromRow)]
pub fn derive_from_row(item: TokenStream) -> TokenStream {
    let input = parse_macro_input!(item as DeriveInput);
    match from_row::expand(&input) {
        Ok(tokens) => TokenStream::from(tokens),
        Err(err) => TokenStream::from(err.into_compile_error()),
    }
}
