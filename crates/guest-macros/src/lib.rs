#![doc = include_str!("../README.md")]

//! Procedural attributes for Omnia guests.

mod operation;
mod otel;

use proc_macro::TokenStream;
use quote::quote;
use syn::{ItemFn, meta, parse_macro_input};

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
            let _guard = ::omnia_wasi_otel::init();
            #body
        }
    };

    TokenStream::from(new_fn)
}

/// Derives an `omnia_guest::api::Operation` implementation from a bare
/// handler function.
///
/// The function must be an `async fn` with exactly two parameters — the owned
/// operation input and a `CallContext<'_, P>` — returning `Result<T>`
/// (`omnia_guest::Result`) or `Result<T, E>`. The macro re-emits the function
/// unchanged (attributes included, so a `#[tracing::instrument]` on the fn
/// keeps working) and generates `impl<P> Operation<P> for <InputType>`
/// reusing the fn's generics and bounds (which must include `P: Provider`,
/// as `CallContext` already requires); the generated `call` delegates to the
/// fn.
#[proc_macro_attribute]
pub fn operation(args: TokenStream, item: TokenStream) -> TokenStream {
    if !args.is_empty() {
        return syn::Error::new_spanned(
            proc_macro2::TokenStream::from(args),
            "#[operation] takes no arguments",
        )
        .into_compile_error()
        .into();
    }

    let item_fn = parse_macro_input!(item as ItemFn);
    operation::expand(&item_fn).unwrap_or_else(syn::Error::into_compile_error).into()
}
