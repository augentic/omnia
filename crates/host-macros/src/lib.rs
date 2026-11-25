#![doc = include_str!("../README.md")]

//! Procedural macros for the omnia host runtime.

mod from_env;
mod host_ctx;
mod runtime;

use proc_macro::TokenStream;
use syn::{DeriveInput, parse_macro_input};

/// Generates the runtime infrastructure based on the configuration.
///
/// # Examples
///
/// ```rust,ignore
/// use omnia_redis::Client as Redis;
/// use omnia_wasi_http::{HttpDefault, WasiHttp};
/// use omnia_wasi_keyvalue::WasiKeyValue;
/// use omnia_wasi_otel::{OtelDefault, WasiOtel};
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
/// `Data<'a> = omnia_core::HostView<'a, dyn Client>` and `omnia_core::HostCtx`
/// with `Borrow<'a> = &'a dyn Client`. The host's bindgen `Host*WithStore`
/// functions borrow that view inside `Accessor::with`.
///
/// The type must be non-generic and name exactly one `client`.
///
/// The emitted paths start at `::omnia_core`. A crate that reaches the derive
/// through a re-export names that root in `crate`, as in
/// `#[host_ctx(client = FooClient, crate = omnia)]`.
///
/// # Examples
///
/// ```rust,ignore
/// use omnia_core::{Host, HostCtx, Provides, StoreCtx};
/// use wasmtime::component::Linker;
///
/// /// Host-side service for `omnia:keyvalue`.
/// #[derive(Debug, HostCtx)]
/// #[host_ctx(client = KeyValueClient)]
/// pub struct WasiKeyValue;
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

/// Derives `omnia_core::FromEnv` for a struct of options read from
/// environment variables.
///
/// Each field takes an `#[env(..)]` attribute in one of three shapes:
///
/// - `from = "NAME"` reads the variable `NAME`. When it is unset, the field
///   takes its `default = ".."`, an `Option` field is `None`, and any other
///   field fails the load.
/// - `default = ".."` alone marks a field set in code, which no variable
///   reads.
/// - `nested` loads a field whose type derives `FromEnv`. An `Option` field
///   is `None` when every problem inside it is an unset variable.
///
/// A value parses through its type's `FromStr`, or through the `with = path`
/// the field names: any `fn(&str) -> Result<T, E>` whose `E` implements
/// `Display`. An `Option<T>` field parses a `T` and takes no `default`.
///
/// The struct takes `#[env(validate = path)]`, a
/// `fn(&Self) -> anyhow::Result<()>` every load runs once its fields parse.
///
/// The derive emits:
///
/// - `load_env()`, `with_env(self)`, and `load_from(lookup)`, which load from
///   the process environment, over a value set in code, and through a lookup.
///   A load fails with one error naming every required variable that is unset
///   and every variable whose value does not parse, but never a value.
/// - `omnia_core::FromEnv`, over `load_env`.
/// - `Default`, where every field has a `default` or is an `Option`. A
///   `default` that does not parse panics there, naming the field.
///
/// The type must be a non-generic struct with named fields.
///
/// The emitted paths start at `::omnia_core`. A crate that reaches the derive
/// through a re-export names that root in `crate`, as in
/// `#[env(crate = omnia)]`.
///
/// # Examples
///
/// ```rust,ignore
/// use omnia::FromEnv;
///
/// /// Options for the Redis backend.
/// #[derive(Clone, Debug, FromEnv)]
/// #[env(crate = omnia)]
/// pub struct ConnectOptions {
///     /// Redis URL (`REDIS_URL`, default `redis://localhost:6379`).
///     #[env(from = "REDIS_URL", default = "redis://localhost:6379")]
///     pub url: String,
///     /// Buckets a guest may open (`KEYVALUE_BUCKETS`, comma-separated).
///     #[env(from = "KEYVALUE_BUCKETS", default = "default", with = omnia::env::list)]
///     pub buckets: Vec<String>,
/// }
///
/// let options = ConnectOptions::load_env()?;
/// ```
#[proc_macro_derive(FromEnv, attributes(env))]
pub fn derive_from_env(item: TokenStream) -> TokenStream {
    let input = parse_macro_input!(item as DeriveInput);
    match from_env::expand(&input) {
        Ok(tokens) => TokenStream::from(tokens),
        Err(err) => TokenStream::from(err.into_compile_error()),
    }
}
