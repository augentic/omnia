#![doc = include_str!("../README.md")]
#![cfg(not(target_arch = "wasm32"))]

mod artifact;
mod chain;
mod digest;
mod dispatch;
mod egress;
pub mod env;
mod extensions;
mod grants;
mod host;
mod invoke;
mod mount;
mod names;
mod options;
mod registry;
mod runtime;
mod seam;
mod source;
mod store;
pub mod telemetry;
mod value;

pub use omnia_compile::CompileOptions;
pub use omnia_host_macros::{FromEnv, HostCtx};
pub use tracing_subscriber::filter::LevelFilter;
#[doc(hidden)]
pub use {anyhow, futures, tokio, wasmtime, wasmtime_wasi};

pub use self::chain::{ChainCtx, ChainPolicy};
pub use self::digest::Digest;
pub use self::dispatch::Dispatcher;
pub use self::egress::host_pattern;
pub use self::extensions::Extensions;
pub use self::grants::Grants;
pub use self::host::{
    Backends, Connect, FromEnv, FutureResult, HasTable, Host, HostCtx, HostView, NoOptions,
    Provides, Proxy, get_cloned,
};
pub use self::invoke::{InvokeError, PoolExhausted, PoolResource};
pub use self::mount::{MountRegistry, ResolvedPreopen};
pub use self::options::RuntimeOptions;
pub use self::registry::{
    Guest, GuestId, HttpRoutes, PatternRoutes, Registry, RegistryParts, Resolver, Routes,
    TriggerRouter,
};
pub use self::runtime::{
    Bound, Entered, ExitStatus, GuestError, Invocations, Lease, LeaseError, Policy, Runtime,
    RuntimeParts, WeakRuntime,
};
pub use self::seam::{LinkSeam, NoLinks};
pub use self::source::{AcquireError, LoadedGuest, RegistrySource, Source, SourceSpec, Verified};
pub use self::store::{Stdio, StoreBase, StoreCtx};
pub use self::telemetry::Telemetry;
pub use self::value::handle_kind;
