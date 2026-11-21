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

pub use omnia_compile::CompileOptions;
pub use omnia_host_macros::HostCtx;
pub use tracing_subscriber::filter::LevelFilter;
#[doc(hidden)]
pub use {anyhow, futures, tokio, wasmtime, wasmtime_wasi};

pub use self::chain::{ChainCtx, ChainPolicy};
pub use self::digest::Digest;
pub use self::dispatch::Dispatcher;
pub use self::extensions::Extensions;
pub use self::host::{
    Backends, Connect, FromEnv, FutureResult, HasTable, Host, HostCtx, HostView, NoOptions,
    Provides, Proxy, get_cloned,
};
pub use self::invoke::{FreshCall, InvokeError, call_fresh};
pub use self::mount::{MountRegistry, ResolvedPreopen};
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
