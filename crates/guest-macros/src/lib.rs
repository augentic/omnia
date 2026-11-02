#![doc = include_str!("../README.md")]

//! Procedural attributes for Omnia guests.

mod otel;
mod provider;

use proc_macro::TokenStream;
use quote::quote;
use syn::{DeriveInput, ItemFn, meta, parse_macro_input};

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

/// Derives `omnia_sdk` capability traits by delegating each named capability to
/// the struct field annotated with `#[provider(Capability, ...)]`.
#[proc_macro_derive(Provider, attributes(provider))]
pub fn derive_provider(item: TokenStream) -> TokenStream {
    let input = parse_macro_input!(item as DeriveInput);
    match provider::expand(&input) {
        Ok(tokens) => TokenStream::from(tokens),
        Err(err) => TokenStream::from(err.into_compile_error()),
    }
}
