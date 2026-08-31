#![doc = include_str!("../README.md")]
#![cfg(not(target_arch = "wasm32"))]
#![allow(unsafe_code)] // wasmtime component deserialization and deployment hooks

#[cfg(feature = "cli")]
mod cli;
mod deployment;
mod dispatch;
mod host;
mod mount;
mod options;
mod plugins;
mod registry;
mod runtime;
mod store;
pub mod telemetry;

#[cfg(feature = "cli")]
pub use clap::Parser;
pub use omnia_host_macros::runtime;
#[doc(hidden)]
pub use pastey;
#[doc(hidden)]
pub use wrpc_wasmtime::{WrpcCtxView, WrpcView};
#[doc(hidden)]
pub use {anyhow, futures, tokio, wasmtime, wasmtime_wasi};

#[cfg(feature = "cli")]
pub use self::cli::{Cli, Command};
pub use self::deployment::{
    Deployment, DeploymentBuilder, GuestArtifact, GuestEntry, GuestRoutes, Manifest, Mount,
    Precompiled, SourceSpec, Transport, TransportKind, WasmOnly,
};
pub use self::dispatch::{
    Dispatcher, FirstArgSelector, GuestSelector, LinkClient, WrpcState, as_command_chain,
    serve_links,
};
pub use self::host::{
    Backend, FromEnv, FutureResult, HasTable, Host, HostCtx, NoOptions, Provides, Proxy, Server,
    get_cloned,
};
pub use self::mount::{MountRegistry, ResolvedPreopen};
pub use self::options::RuntimeOptions;
#[cfg(feature = "jit")]
pub use self::options::compile;
pub use self::plugins::{
    Acquire, AcquireContext, AcquireError, LoadError, Location, MountAcquire, MountEntry, Plugin,
    PluginLoader, WasiPlugins, WasiPluginsCtxView, WasiPluginsView,
};
pub use self::registry::{
    CliRoutes, Guest, GuestId, HttpRoutes, PatternRoutes, Registry, Routes, TriggerRouter,
};
pub use self::runtime::{Backends, ExitStatus, Mode, Runtime, Wiring};
#[doc(hidden)]
pub use self::runtime::{MainOptions, ManifestSource, main, run, run_precompiled};
pub use self::store::{
    HasDispatcher, HasLimits, HasMounts, HttpBorrow, HttpCtx, StoreBase, StoreConfig, StoreCtx,
    StoreView,
};
pub use self::telemetry::{LogMode, Telemetry};

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
/// Emits the `Wasi<Service>CtxView` borrowed `(ctx, table)` view plus the
/// [`HostCtx`] impl on the `Wasi<Service>` host type, so the generic
/// `omnia::StoreView` blanket serves the host's `add_to_linker` accessor
/// with no per-host view or accessor trait.
///
/// Pass the service stem (the part after `Wasi` in the host struct name). The
/// `Wasi<Service>Ctx` trait, the `HasData` impl, the `bindgen!` block, and the
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
            #[doc = concat!("Borrowed view over a [`", stringify!([<Wasi $name Ctx>]), "`] and the store's resource table.")]
            pub struct [<Wasi $name CtxView>]<'a> {
                #[doc = concat!("Mutable reference to the WASI ", stringify!($name), " context.")]
                pub ctx: &'a mut dyn [<Wasi $name Ctx>],
                /// Mutable reference to the table used to manage resources.
                pub table: &'a mut $crate::wasmtime_wasi::ResourceTable,
            }

            impl $crate::HasTable for [<Wasi $name CtxView>]<'_> {
                fn table(&mut self) -> &mut $crate::wasmtime_wasi::ResourceTable {
                    self.table
                }
            }

            impl $crate::HostCtx for [<Wasi $name>] {
                type Borrow<'a> = &'a mut dyn [<Wasi $name Ctx>];

                fn view<'a>(
                    borrow: Self::Borrow<'a>,
                    table: &'a mut $crate::wasmtime_wasi::ResourceTable,
                ) -> Self::Data<'a> {
                    [<Wasi $name CtxView>] { ctx: borrow, table }
                }
            }
        }
    };
}
