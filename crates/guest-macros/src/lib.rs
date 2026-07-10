#![doc = include_str!("../README.md")]

//! Procedural macros for the omnia guest.

#![forbid(unsafe_code)]

mod command;
mod guest;
mod http;
mod messaging;
mod otel;

use proc_macro::TokenStream;
use quote::quote;
use syn::{ItemFn, meta, parse_macro_input};

/// Generates the guest infrastructure based on the specified configuration.
///
/// The `http:` table expands to a target-neutral `http_router(client)`
/// function over [`omnia_guest::api::route`] constructors (path, query,
/// and JSON-body inputs deserialize into each request's `Handler::Input`),
/// plus a wasm-gated `wasi:http` export serving it. The `command:` arm
/// wires an app-supplied async dispatch function
/// (`async fn(Vec<String>) -> u8`) to a generated `wasi:cli/run` export
/// with argv fetch and exit-code passthrough.
///
/// The provider is constructed once and held in a `static` for the
/// component's lifetime, so it must be `Sync` (trivially satisfied on the
/// single-threaded wasm target unless it holds `!Sync` interior mutability
/// such as `RefCell` — wrap such state in a `Mutex` instead).
///
/// # Example
///
/// ```rust,ignore
/// omnia_guest_macros::guest!({
///     owner: "at",
///     provider: MyProvider,
///     http: [
///         "/some/get/path": get(SomeRequest, SomeResponse),
///         "/some/other-get/path": get(SomeRequest with_query, SomeResponse),
///         "/some/post/path": post(SomeRequest, SomeResponse),
///         "/some/post-body/path": post(SomeRequest with_body, SomeResponse),
///     ],
///     messaging: [
///         "topic-name.v1": TopicMessage,
///         "other-topic.v2": OtherTopicMessage,
///     ],
///     command: dispatch,
/// });
/// ```
#[proc_macro]
pub fn guest(input: TokenStream) -> TokenStream {
    let config = parse_macro_input!(input as guest::Config);
    guest::expand(&config).into()
}

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
