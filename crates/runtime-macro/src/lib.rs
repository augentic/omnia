#![doc = include_str!("../README.md")]

//! Procedural macros for the omnia runtime.

#![forbid(unsafe_code)]

mod expand;
mod runtime;
mod store_context;

use proc_macro::TokenStream;
use syn::{DeriveInput, parse_macro_input};

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
    match expand::expand(&parsed) {
        Ok(ts) => ts.into(),
        Err(e) => e.into_compile_error().into(),
    }
}

/// Derives the fixed store-context trait impls for a `StoreCtx`.
///
/// Implements `WasiView`, `WrpcView`, and `HasLimits` against the `#[base]`
/// field (of type [`omnia::StoreBase`]) and emits one host view per `#[wasi(path)]`
/// backend field via that host crate's `omnia_wasi_view!` macro.
///
/// # Example
///
/// ```rust,ignore
/// #[derive(omnia::StoreContext)]
/// struct StoreCtx {
///     #[base]
///     base: omnia::StoreBase,
///     #[wasi(omnia_wasi_http)]
///     http: HttpDefault,
/// }
/// ```
#[proc_macro_derive(StoreContext, attributes(base, wasi))]
pub fn store_context(input: TokenStream) -> TokenStream {
    let input = parse_macro_input!(input as DeriveInput);
    match store_context::expand(&input) {
        Ok(ts) => ts.into(),
        Err(e) => e.into_compile_error().into(),
    }
}
