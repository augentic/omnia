#![doc = include_str!("../README.md")]

//! Procedural macros for the omnia host runtime.

mod host_ctx;
mod runtime;

use proc_macro::TokenStream;
use syn::{DeriveInput, parse_macro_input};

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

/// Derives `omnia_core::HostCtx` for a `Wasi<Service>` host over its `<Service>Client`.
///
/// For `Wasi<Service>`, the derive emits:
///
/// - `Wasi<Service>View<'a>`, the borrowed `{ client, table }` view the
///   bindgen `Host*` traits are implemented on, plus `omnia_core::HasTable`
///   so `omnia_core::get_cloned` reaches the resource table.
/// - `wasmtime::component::HasData` naming `Wasi<Service>View<'a>` as the
///   host's `Data<'a>`.
/// - `omnia_core::HostCtx` with `Borrow<'a> = &'a mut dyn <Service>Client`,
///   assembling `Wasi<Service>View { client, table }`.
///
/// Together with the `Provides<Wasi<Service>>` impl that [`runtime!`] emits
/// on the generated `Backends` bundle, this satisfies the blanket
/// `StoreView<H> for StoreCtx<B>` impl in `omnia-core`:
///
/// ```text
/// StoreCtx<Backends>::view(&mut self) -> Wasi<Service>View<'_>
///   └── Wasi<Service>::view(self.backends.borrow(), &mut self.base.table)
/// ```
///
/// A host crate bounds its `Host<T>` impl on `T: StoreView<Self>` and passes
/// `T::view` to bindgen's `add_to_linker`.
///
/// The type must be a non-generic `Wasi<Service>`, `<Service>Client` must be
/// in scope, and the crate must depend on `omnia-core`.
///
/// # Examples
///
/// ```rust,ignore
/// pub use omnia_backend::keyvalue::KeyValueClient;
/// use omnia_core::{Host, HostCtx, StoreView};
/// use wasmtime::component::Linker;
///
/// /// Host-side service for `wasi:keyvalue`.
/// #[derive(Debug, HostCtx)]
/// pub struct WasiKeyValue;
///
/// impl<T> Host<T> for WasiKeyValue
/// where
///     T: StoreView<Self> + 'static,
/// {
///     fn add_to_linker(linker: &mut Linker<T>) -> anyhow::Result<()> {
///         // `T::view` projects `StoreCtx<B>` into `WasiKeyValueView<'_>`.
///         Ok(store::add_to_linker::<_, Self>(linker, T::view)?)
///     }
/// }
/// ```
#[proc_macro_derive(HostCtx)]
pub fn derive_host_ctx(item: TokenStream) -> TokenStream {
    let input = parse_macro_input!(item as DeriveInput);
    match host_ctx::expand(&input) {
        Ok(tokens) => TokenStream::from(tokens),
        Err(err) => TokenStream::from(err.into_compile_error()),
    }
}
