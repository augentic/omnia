#![doc = include_str!("../README.md")]

//! Procedural macros for the omnia host runtime.

#![forbid(unsafe_code)]

mod runtime;

use proc_macro::TokenStream;
use syn::parse_macro_input;

/// Generates the runtime infrastructure based on the configuration.
///
/// # Example
///
/// ```rust,ignore
/// omnia::runtime!({
///     omnia_wasi_http: WasiHttp,
///     omnia_wasi_otel: DefaultOtel,
///     omnia_wasi_blobstore: MongoDb,
/// });
/// ```
#[proc_macro]
pub fn runtime(input: TokenStream) -> TokenStream {
    let parsed = parse_macro_input!(input as runtime::Config);
    runtime::expand(&parsed).into()
}
