#![doc = include_str!("../README.md")]
#![cfg(not(target_arch = "wasm32"))]

mod cli;
mod deployment;
mod dispatch;
mod mount;
mod options;
mod registry;
mod runtime;
mod store;
mod telemetry;
mod traits;

pub use clap::Parser;
pub use omnia_host_macros::runtime;
#[doc(hidden)]
pub use pastey;
#[doc(hidden)]
pub use wrpc_wasmtime::{WrpcCtxView, WrpcView};
#[doc(hidden)]
pub use {anyhow, futures, tokio, wasmtime, wasmtime_wasi};

pub use self::cli::{Cli, Command};
pub use self::deployment::{Deployment, DeploymentBuilder};
pub use self::dispatch::{
    Dispatcher, FirstArgSelector, GuestSelector, LinkClient, WrpcState, serve_links,
};
pub use self::mount::{Mount, MountRegistry, ResolvedPreopen};
pub use self::options::RuntimeOptions;
#[cfg(feature = "jit")]
pub use self::options::compile;
pub use self::registry::{
    CliRoutes, Guest, GuestId, HttpRoutes, Registry, Resolver, Routes, TopicRoutes, TriggerRouter,
};
#[doc(hidden)]
pub use self::runtime::assert_hosts;
pub use self::runtime::{ExitStatus, Runtime, RuntimeHooks};
#[doc(hidden)]
pub use self::runtime::{main, run};
pub use self::store::{HasDispatcher, HasHttp, HasMounts, StoreBase, StoreBaseBuilder, StoreCtx};
#[doc(hidden)]
pub use self::store::{Set, Unset};
pub use self::telemetry::{Telemetry, resource};
pub use self::traits::{Backend, Backends, FromEnv, FutureResult, HasLimits, Host, Server};

/// Generates the linker-facing view traits that every `omnia` WASI host crate
/// repeats verbatim (only the names change):
///
/// - `Wasi<Service>View`: the per-`Linker<T>` accessor trait,
/// - `Wasi<Service>CtxView`: the borrowed `(ctx, table)` view,
/// - `Has<Service>`: the backend-bundle accessor trait,
/// - the blanket `Wasi<Service>View for omnia::StoreCtx<B>` impl.
///
/// Pass the service stem (the part after `Wasi` in the host struct name). All
/// identifiers and doc labels are derived from it: `KeyValue` yields
/// `WasiKeyValueView`, `HasKeyValue`, `keyvalue`, `keyvalue_ctx`, and doc text
/// using `stringify!(KeyValue)`.
///
/// The service-specific pieces stay hand-written in each crate: the
/// `Wasi<Service>Ctx` trait, the `bindgen!` block, the `Host`/`Server` wiring,
/// the error conversions, and the `omnia_wasi_view!` macro (whose `$crate` must
/// resolve to the host crate, which a macro-generated macro cannot express).
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
            /// impl via `omnia_wasi_view!`.
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
