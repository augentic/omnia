#![doc = include_str!("../README.md")]

//! Procedural macros for the omnia guest.

#![forbid(unsafe_code)]

mod guest;
mod http;
mod messaging;
mod otel;

use proc_macro::TokenStream;
use quote::quote;
use syn::{ItemFn, meta, parse_macro_input};

/// Generates the guest infrastructure based on the specified configuration.
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
///     ]
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
    let signature = &item_fn.sig;
    let body = otel::body(attrs, &item_fn);

    // recreate function with the instrument macro wrapping the body
    let new_fn = quote! {
        #signature {
            let _guard = ::omnia_wasi_otel::init();
            #body
        }
    };

    TokenStream::from(new_fn)
}
