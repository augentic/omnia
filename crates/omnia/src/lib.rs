#![doc = include_str!("../README.md")]
#![cfg(not(target_arch = "wasm32"))]
#![allow(unsafe_code)] // wasmtime component deserialization and deployment hooks

mod cli;
mod deployment;
mod dispatch;
mod host;
mod mount;
mod options;
mod registry;
mod runtime;
mod store;
mod telemetry;

pub use clap::Parser;
pub use omnia_host_macros::runtime;
#[doc(hidden)]
pub use pastey;
#[doc(hidden)]
pub use wrpc_wasmtime::{WrpcCtxView, WrpcView};
#[doc(hidden)]
pub use {anyhow, futures, tokio, wasmtime, wasmtime_wasi};

pub use self::cli::{Cli, Command};
pub use self::deployment::{Deployment, DeploymentBuilder, Mount};
pub use self::dispatch::{
    Dispatcher, FirstArgSelector, GuestSelector, LinkClient, WrpcState, serve_links,
};
pub use self::host::{Backend, FromEnv, FutureResult, Host, Server};
pub use self::mount::{MountRegistry, ResolvedPreopen};
pub use self::options::RuntimeOptions;
#[cfg(feature = "jit")]
pub use self::options::compile;
pub use self::registry::{
    CliRoutes, GuestId, HttpRoutes, PatternRoutes, Registry, Routes, TriggerRouter,
};
pub use self::runtime::{Backends, ExitStatus, Mode, Runtime, Wiring};
#[doc(hidden)]
pub use self::runtime::{main, run};
pub use self::store::{
    HasDispatcher, HasHttp, HasLimits, HasMounts, StoreBase, StoreBaseBuilder, StoreCtx,
};
pub use self::telemetry::{Telemetry, resource};

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
/// Emits the `Wasi<Service>View` accessor trait, the `Wasi<Service>CtxView`
/// borrowed `(ctx, table)` view, the `Has<Service>` backend-accessor trait, and
/// the blanket `Wasi<Service>View for omnia::StoreCtx<B>` impl.
///
/// Pass the service stem (the part after `Wasi` in the host struct name); every
/// identifier is derived from it. The `Wasi<Service>Ctx` trait, `bindgen!`
/// block, and `Host`/`Server` wiring stay hand-written; error conversions come
/// from [`host_error!`].
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
            #[doc = concat!("Provides internal WASI ", stringify!($name), " state.")]
            ///
            /// Implemented by the `T` in `Linker<T>`: a single type shared across
            /// every WASI component in a runtime build.
            pub trait [<Wasi $name View>]: Send {
                #[doc = concat!("Borrow a `", stringify!([<Wasi $name CtxView>]), "` from a mutable reference to self.")]
                fn [<$name:lower>](&mut self) -> [<Wasi $name CtxView>]<'_>;
            }

            #[doc = concat!("Borrowed view over a [`", stringify!([<Wasi $name Ctx>]), "`] and the store's resource table.")]
            pub struct [<Wasi $name CtxView>]<'a> {
                #[doc = concat!("Mutable reference to the WASI ", stringify!($name), " context.")]
                pub ctx: &'a mut dyn [<Wasi $name Ctx>],
                /// Mutable reference to the table used to manage resources.
                pub table: &'a mut $crate::wasmtime_wasi::ResourceTable,
            }

            #[doc = concat!("A backend bundle that yields the WASI ", stringify!($name), " context for a store.")]
            ///
            /// The blanket view impl turns this accessor into the linker-facing view
            /// on `omnia::StoreCtx`; `runtime!` deployments generate the bundle-side
            /// impl directly.
            pub trait [<Has $name>]: Send {
                #[doc = concat!("Borrow the WASI ", stringify!($name), " backend context.")]
                fn [<$name:lower _ ctx>](&mut self) -> &mut dyn [<Wasi $name Ctx>];
            }

            impl<B: [<Has $name>] + Send + 'static> [<Wasi $name View>] for $crate::StoreCtx<B> {
                fn [<$name:lower>](&mut self) -> [<Wasi $name CtxView>]<'_> {
                    [<Wasi $name CtxView>] {
                        ctx: self.backends.[<$name:lower _ ctx>](),
                        table: &mut self.base.table,
                    }
                }
            }
        }
    };
}
