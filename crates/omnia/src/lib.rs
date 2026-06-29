#![doc = include_str!("../README.md")]
#![cfg(not(target_arch = "wasm32"))]

mod cli;
mod deployment;
mod dispatch;
mod options;
mod registry;
mod runtime;
mod store;
mod telemetry;
mod traits;
mod working_tree;

pub use clap::Parser;
pub use omnia_host_macros::runtime;
#[doc(hidden)]
pub use wrpc_wasmtime::{WrpcCtxView, WrpcView};
#[doc(hidden)]
pub use {anyhow, futures, tokio, wasmtime, wasmtime_wasi};

#[cfg(feature = "jit")]
pub use self::options::compile;
pub use self::cli::{Cli, Command};
pub use self::deployment::{Deployment, DeploymentBuilder};
pub use self::dispatch::{
    FirstArgSelector, GuestSelector, HostDispatch, LinkClient, WrpcState, serve_links,
};
pub use self::options::RuntimeOptions;
pub use self::registry::{Guest, GuestId, Registry};
pub use self::registry::{CliRoutes, HttpRoutes, Resolver, Routes, TopicRoutes, TriggerRouter};
pub use self::runtime::{ExitStatus, Runtime, RuntimeHooks};
#[doc(hidden)]
pub use self::runtime::{main, run};
pub use self::store::{HasHttp, StoreBase, StoreBaseBuilder, StoreCtx};
#[doc(hidden)]
pub use self::store::{Set, Unset};
pub use self::telemetry::{Telemetry, resource};
#[doc(hidden)]
pub use self::traits::assert_hosts;
pub use self::traits::{Backend, Backends, FromEnv, FutureResult, HasLimits, Host, Server};
pub use self::working_tree::{ResolvedPreopen, WorkingTreeEntry, WorkingTreeRegistry};

/// Generates the linker-facing view scaffold that every `omnia` WASI host crate
/// repeats verbatim (only the names change):
///
/// - `Wasi<Service>View`: the per-`Linker<T>` accessor trait,
/// - `Wasi<Service>CtxView`: the borrowed `(ctx, table)` view,
/// - `Has<Service>`: the backend-bundle accessor trait,
/// - the blanket `Wasi<Service>View for omnia::StoreCtx<B>` impl.
///
/// The service-specific pieces stay hand-written in each crate: the
/// `Wasi<Service>Ctx` trait, the `bindgen!` block, the `Host`/`Server` wiring,
/// the error conversions, and the `omnia_wasi_view!` macro (whose `$crate` must
/// resolve to the host crate, which a macro-generated macro cannot express).
///
/// # Example
///
/// ```ignore
/// omnia::scaffold! {
///     service: "Key-Value",
///     view: WasiKeyValueView,
///     ctx: WasiKeyValueCtx,
///     ctx_view: WasiKeyValueCtxView,
///     backend: HasKeyValue,
///     accessor: keyvalue,
///     ctx_accessor: keyvalue_ctx,
/// }
/// ```
#[macro_export]
macro_rules! scaffold {
    (
        service: $label:literal,
        view: $view:ident,
        ctx: $ctx:ident,
        ctx_view: $ctx_view:ident,
        backend: $has:ident,
        accessor: $accessor:ident,
        ctx_accessor: $ctx_accessor:ident
        $(,)?
    ) => {
        #[doc = concat!("Provides internal WASI ", $label, " state.")]
        ///
        /// Implemented by the `T` in `Linker<T>`: a single type shared across
        /// every WASI component in a runtime build.
        pub trait $view: Send {
            #[doc = concat!("Borrow a `", stringify!($ctx_view), "` from a mutable reference to self.")]
            fn $accessor(&mut self) -> $ctx_view<'_>;
        }

        #[doc = concat!("Borrowed view over a `", stringify!($ctx), "` and the store's resource table.")]
        pub struct $ctx_view<'a> {
            #[doc = concat!("Mutable reference to the WASI ", $label, " context.")]
            pub ctx: &'a mut dyn $ctx,
            /// Mutable reference to the table used to manage resources.
            pub table: &'a mut $crate::wasmtime_wasi::ResourceTable,
        }

        #[doc = concat!("A backend bundle that yields the WASI ", $label, " context for a store.")]
        ///
        /// The blanket view impl turns this accessor into the linker-facing view
        /// on `omnia::StoreCtx`; `runtime!` deployments generate the bundle-side
        /// impl via `omnia_wasi_view!`.
        pub trait $has: Send {
            #[doc = concat!("Borrow the WASI ", $label, " backend context.")]
            fn $ctx_accessor(&mut self) -> &mut dyn $ctx;
        }

        impl<B: $has + Send + 'static> $view for $crate::StoreCtx<B> {
            fn $accessor(&mut self) -> $ctx_view<'_> {
                $ctx_view {
                    ctx: self.backends.$ctx_accessor(),
                    table: &mut self.base.table,
                }
            }
        }
    };
}
