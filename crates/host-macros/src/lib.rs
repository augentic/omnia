#![doc = include_str!("../README.md")]

//! Procedural macros for the omnia host runtime.

#![forbid(unsafe_code)]

mod runtime;
mod runtime_derive;

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
    runtime::expand(&parsed).into()
}

/// Derives the standard `omnia::Runtime` impl for a deployment runtime.
///
/// Generates `type StoreCtx`, the `registry()` accessor, and a `store()` that
/// builds the fixed `base: omnia::StoreBase` plus one cloned backend per
/// `#[runtime(store = ...)]` field. The struct must implement `Clone` (the
/// `Runtime` supertrait already requires it) and carry its `StoreBase` in a
/// field named `base` on the target `StoreCtx`.
///
/// # Example
///
/// ```rust,ignore
/// #[derive(Clone, omnia::Runtime)]
/// #[runtime(store = StoreCtx)]
/// struct Context {
///     #[runtime(registry)]
///     registry: Arc<Registry<StoreCtx>>,
///     #[runtime(store = omnia_wasi_http)]
///     http_default: HttpDefault,
/// }
/// ```
#[proc_macro_derive(Runtime, attributes(runtime))]
pub fn runtime_derive(input: TokenStream) -> TokenStream {
    let input = parse_macro_input!(input as DeriveInput);
    match runtime_derive::expand(&input) {
        Ok(ts) => ts.into(),
        Err(e) => e.into_compile_error().into(),
    }
}
