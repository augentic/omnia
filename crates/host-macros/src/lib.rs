#![doc = include_str!("../README.md")]

//! Procedural macros for the omnia host runtime.

mod host_ctx;
mod runtime;

use proc_macro::TokenStream;
use syn::{DeriveInput, parse_macro_input};

/// Generates a host runtime from a map of `Host: Backend` pairs, or a list
/// of backends.
///
/// In the map, each key is a host type from an `omnia-wasi-*` crate and each
/// value a backend implementing that interface's context trait. In the list,
/// each backend names the hosts it serves. The optional `mode:`, `manifest:`,
/// and inline manifest keys are documented at the crate root.
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

/// Derives `omnia_core::HostCtx` for a host over the client trait
/// `#[host_ctx(client = ..)]` names.
///
/// The derive emits `wasmtime::component::HasData` with
/// `Data<'a> = omnia_core::HostView<'a, dyn Client>`, `omnia_core::HostCtx`
/// with `Borrow<'a> = &'a dyn Client`, and `omnia_core::SlotHost` over the
/// slot `Arc<dyn Client>`. The host implements its bindgen `Host*` traits on
/// `HostView<'_, dyn Client>`.
///
/// The type must be non-generic, carry exactly one `#[host_ctx(client = ..)]`
/// and a `capability!` line, and the crate must depend on `omnia-core`.
///
/// # Examples
///
/// ```rust,ignore
/// use omnia_core::{Host, HostCtx, Provides, StoreCtx};
/// use wasmtime::component::Linker;
///
/// use crate::KeyValueClient;
///
/// /// Host-side service for `wasi:keyvalue`.
/// #[derive(Debug, HostCtx)]
/// #[host_ctx(client = KeyValueClient)]
/// pub struct WasiKeyValue;
///
/// omnia_backend::capability!(WasiKeyValue, "wasi:keyvalue", KeyValueClient);
///
/// impl<B: Provides<Self>> Host<B> for WasiKeyValue {
///     fn add_to_linker(linker: &mut Linker<StoreCtx<B>>) -> anyhow::Result<()> {
///         Ok(store::add_to_linker::<_, Self>(linker, StoreCtx::view::<Self>)?)
///     }
/// }
/// ```
#[proc_macro_derive(HostCtx, attributes(host_ctx))]
pub fn derive_host_ctx(item: TokenStream) -> TokenStream {
    let input = parse_macro_input!(item as DeriveInput);
    match host_ctx::expand(&input) {
        Ok(tokens) => TokenStream::from(tokens),
        Err(err) => TokenStream::from(err.into_compile_error()),
    }
}
