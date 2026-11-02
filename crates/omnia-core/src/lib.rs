#![doc = include_str!("../README.md")]
#![cfg(not(target_arch = "wasm32"))]

mod artifact;
mod chain;
mod digest;
mod dispatch;
mod extensions;
mod host;
mod invoke;
mod options;
mod registry;
mod runtime;
mod seam;
mod source;
mod store;
mod value;

pub use omnia_backend::{
    Connect, FromEnv, FutureResult, LevelFilter, Mount, MountRegistry, NoOptions, ResolvedPreopen,
    Telemetry, telemetry,
};
pub use omnia_compile::CompileOptions;
#[doc(hidden)]
pub use pastey;
#[doc(hidden)]
pub use {anyhow, futures, tokio, wasmtime, wasmtime_wasi};

pub use self::chain::{ChainCtx, ChainPolicy};
pub use self::digest::Digest;
pub use self::dispatch::Dispatcher;
pub use self::extensions::Extensions;
pub use self::host::{HasTable, Host, HostCtx, Provides, Proxy, Server, get_cloned};
pub use self::invoke::{FreshCall, InvokeError, call_fresh};
pub use self::options::RuntimeOptions;
pub use self::registry::{
    CliRoutes, Guest, GuestId, HttpRoutes, PatternRoutes, Registry, RegistryParts, Resolver,
    Routes, TriggerRouter,
};
pub use self::runtime::{
    ExitStatus, GuestError, Policy, Runtime, RuntimeParts, WeakRuntime, serve_links,
};
pub use self::seam::{LinkSeam, NoLinks, StoreFactory};
pub use self::source::{AcquireError, LoadedGuest, RegistrySource, Source, SourceSpec, Verified};
pub use self::store::{
    HasChain, HasDispatcher, HasExtensions, HasLimits, HasMounts, Stdio, StoreBase, StoreConfig,
    StoreCtx, StoreView,
};
pub use self::value::handle_kind;

/// Generates the standard host-error conversions every `omnia` WASI host
/// crate repeats.
///
/// Emits `From` impls for [`anyhow::Error`], [`wasmtime::Error`], and
/// [`wasmtime::component::ResourceTableError`] into the given string-carrying
/// variant of the WIT-generated error type, preserving the full context chain
/// (`{err:#}`) from backend errors.
///
/// # Example
///
/// ```ignore
/// omnia::host_error!(Error, Other);
/// ```
#[macro_export]
macro_rules! host_error {
    ($error:ty, $variant:ident $(,)?) => {
        impl ::core::convert::From<$crate::anyhow::Error> for $error {
            fn from(err: $crate::anyhow::Error) -> Self {
                // `:#` keeps the full context chain from backend errors.
                Self::$variant(format!("{err:#}"))
            }
        }

        impl ::core::convert::From<$crate::wasmtime::Error> for $error {
            fn from(err: $crate::wasmtime::Error) -> Self {
                Self::$variant(format!("{err:#}"))
            }
        }

        impl ::core::convert::From<$crate::wasmtime::component::ResourceTableError> for $error {
            fn from(err: $crate::wasmtime::component::ResourceTableError) -> Self {
                Self::$variant(err.to_string())
            }
        }
    };
}

/// Generates the linker-facing view boilerplate every `omnia` WASI host crate
/// repeats.
///
/// Emits the `Wasi<Service>View` borrowed `(client, table)` view plus the
/// [`HostCtx`] impl on the `Wasi<Service>` host type, so the generic
/// `omnia::StoreView` blanket serves the host's `add_to_linker` accessor
/// with no per-host view or accessor trait.
///
/// Pass the service stem (the part after `Wasi` in the host struct name). The
/// `<Service>Client` trait, the `HasData` impl, the `bindgen!` block, and the
/// `Host`/`Server` wiring stay hand-written; error conversions come from
/// [`host_error!`].
///
/// # Example
///
/// ```ignore
/// omnia::wasi_view!(KeyValue);
/// ```
#[macro_export]
macro_rules! wasi_view {
    ($name:ident $(,)?) => {
        $crate::pastey::paste! {
            #[doc = concat!("Borrowed view over a [`", stringify!([<$name Client>]), "`] and the store's resource table.")]
            pub struct [<Wasi $name View>]<'a> {
                #[doc = concat!("Mutable reference to the WASI ", stringify!($name), " client.")]
                pub client: &'a mut dyn [<$name Client>],
                /// Mutable reference to the table used to manage resources.
                pub table: &'a mut $crate::wasmtime_wasi::ResourceTable,
            }

            impl $crate::HasTable for [<Wasi $name View>]<'_> {
                fn table(&mut self) -> &mut $crate::wasmtime_wasi::ResourceTable {
                    self.table
                }
            }

            impl $crate::HostCtx for [<Wasi $name>] {
                type Borrow<'a> = &'a mut dyn [<$name Client>];

                fn view<'a>(
                    borrow: Self::Borrow<'a>,
                    table: &'a mut $crate::wasmtime_wasi::ResourceTable,
                ) -> Self::Data<'a> {
                    [<Wasi $name View>] { client: borrow, table }
                }
            }
        }
    };
}
