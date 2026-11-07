#![doc = include_str!("../README.md")]

//! Procedural macros for the omnia host runtime.

mod runtime;

use proc_macro::TokenStream;
use syn::parse_macro_input;

/// Generates a host runtime from a map of `Host: Backend` pairs.
///
/// Each key is a host type from an `omnia-wasi-*` crate and each value a
/// backend implementing that interface's context trait. The optional
/// `mode:`, `manifest:`, and inline manifest keys are documented at the
/// crate root.
///
/// # Examples
///
/// ```rust,ignore
/// use omnia_wasi_http::{HttpDefault, WasiHttp};
/// use omnia_wasi_keyvalue::WasiKeyValue;
/// use omnia_wasi_otel::{OtelDefault, WasiOtel};
/// use omnia_redis::Client as Redis;
///
/// omnia::runtime!({
///     hosts: {
///         WasiHttp: HttpDefault,
///         WasiOtel: OtelDefault,
///         WasiKeyValue: Redis,
///     }
/// });
/// ```
#[proc_macro]
pub fn runtime(input: TokenStream) -> TokenStream {
    let parsed = parse_macro_input!(input as runtime::Config);
    runtime::expand(&parsed).into()
}
