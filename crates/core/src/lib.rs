#![doc = include_str!("../README.md")]
#![cfg(not(target_arch = "wasm32"))]

mod artifact;
mod chain;
mod digest;
mod dispatch;
mod extensions;
mod host;
mod invoke;
mod mount;
mod options;
mod registry;
mod runtime;
mod seam;
mod source;
mod store;
pub mod telemetry;
mod value;

pub use omnia_backend::{Connect, FromEnv, FutureResult, NoOptions};
pub use omnia_compile::CompileOptions;
pub use omnia_host_macros::HostCtx;
pub use tracing_subscriber::filter::LevelFilter;
#[doc(hidden)]
pub use {anyhow, futures, tokio, wasmtime, wasmtime_wasi};

pub use self::chain::{ChainCtx, ChainPolicy};
pub use self::digest::Digest;
pub use self::dispatch::Dispatcher;
pub use self::extensions::Extensions;
pub use self::host::{Backends, HasTable, Host, HostCtx, HostView, Provides, Proxy, get_cloned};
pub use self::invoke::{FreshCall, InvokeError, call_fresh};
pub use self::mount::{Mount, MountRegistry, ResolvedPreopen};
pub use self::options::RuntimeOptions;
pub use self::registry::{
    CliRoutes, Guest, GuestId, HttpRoutes, PatternRoutes, Registry, RegistryParts, Resolver,
    Routes, TriggerRouter,
};
pub use self::runtime::{ExitStatus, GuestError, Policy, Runtime, RuntimeParts, WeakRuntime};
pub use self::seam::{LinkSeam, NoLinks, StoreFactory};
pub use self::source::{AcquireError, LoadedGuest, RegistrySource, Source, SourceSpec, Verified};
pub use self::store::{Stdio, StoreBase, StoreConfig, StoreCtx};
pub use self::telemetry::Telemetry;
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
