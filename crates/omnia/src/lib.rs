#![doc = include_str!("../README.md")]
#![cfg(not(target_arch = "wasm32"))]

// The `runtime!` macro emits `omnia::…` paths, so every name it references
// must stay reachable from here. `#[doc(inline)]` keeps the documented
// surface at `omnia::…` rather than a bare re-export line into the source
// crate.
//
// `anyhow` and `futures` appear in the facade's public signatures, so
// embedders reach them from here; `futures` stays unconditional because the
// macro output uses it whether or not the loader surface is enabled.
mod deployment;
mod entry;
mod lifecycle;
pub use anyhow;
pub use futures;
#[cfg(feature = "jit")]
#[doc(inline)]
pub use omnia_compile as compile;
#[doc(inline)]
pub use omnia_core::{
    AcquireError, Backends, Bound, ChainCtx, ChainPolicy, CompileOptions, Connect, Digest,
    Dispatcher, Entered, ExitStatus, Extensions, FromEnv, FutureResult, Grants, Guest, GuestError,
    GuestId, HasTable, Host, HostCtx, HostView, HttpRoutes, Invocations, Lease, LeaseError,
    LevelFilter, MountRegistry, NoOptions, PatternRoutes, Policy, Provides, Proxy, Registry,
    RegistrySource, ResolvedPreopen, Routes, Runtime, RuntimeOptions, Source, SourceSpec, Stdio,
    StoreBase, StoreCtx, Telemetry, TriggerRouter, Verified, WeakRuntime, env, get_cloned,
    telemetry,
};
#[doc(hidden)]
pub use omnia_core::{tokio, wasmtime, wasmtime_wasi};
#[doc(inline)]
pub use omnia_host_macros::runtime;
#[cfg(feature = "link")]
#[doc(inline)]
pub use omnia_link::is_host;
#[cfg(feature = "loader")]
#[doc(inline)]
pub use omnia_plugin::{
    FsStore, LoadConflict, LoadError, Location, NoStore, PackageStore, Plugin, PluginLoader,
    Plugins, Reference, RegistryClient, WasiPlugins, WasiPluginsView,
};

pub use self::deployment::{
    Deployment, DeploymentBuilder, Env, GuestEntry, GuestRoutes, Manifest, Mount, PluginsConfig,
    RegistryConfig, Transport, TransportKind,
};
#[doc(hidden)]
pub use self::entry::{MainOptions, ManifestSource, main};
pub use self::lifecycle::{ConnectAll, Mode, Wiring};
#[doc(hidden)]
pub use self::lifecycle::{run, run_with};
