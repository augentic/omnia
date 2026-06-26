#![doc = include_str!("../README.md")]

//! Procedural macros for the omnia guest.

#![forbid(unsafe_code)]

mod guest;
mod http;
mod messaging;

use proc_macro::TokenStream;
use syn::parse_macro_input;

/// Generates the guest infrastructure based on the specified configuration.
///
/// # Example
///
/// ```rust,ignore
/// guest_macro::guest!({
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
