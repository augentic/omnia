#![doc = include_str!("../README.md")]

//! Procedural attributes for Omnia guests.

mod from_row;
mod otel;
mod settings;

use proc_macro::TokenStream;
use quote::quote;
use syn::{DeriveInput, ItemFn, meta, parse_macro_input};

/// Opens a span around a fn, and runs an `async fn` inside the telemetry scope.
///
/// Re-exported as `omnia_sdk::otel::instrument`. The span takes the fn's
/// name unless `name = ".."` sets one, at `INFO` unless `level = ..` does.
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

/// Derives a `KEYS` list and an `async fn load()` that reads every field of a
/// named-field struct through `omnia_sdk::config::require`.
///
/// A field's key is its name upper-cased behind the struct's
/// `#[setting(prefix = "..")]`, or the literal in `#[setting(key = "..")]`.
/// An `Option<T>` field reads through `omnia_sdk::config::parse`, so its key
/// may be unset.
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
