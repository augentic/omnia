#![doc = include_str!("../README.md")]
#![cfg(not(target_arch = "wasm32"))]

// The embedder facade: the runtime spine (`omnia-core`), the plugins
// capability (`omnia-plugin`), and the `runtime!` macro, re-exported under
// one root. The `runtime!` macro emits `omnia::…` paths, so every name it
// references must stay reachable from here.

#[cfg(feature = "jit")]
pub use omnia_core::compile;
pub use omnia_core::{
    AdmitError, Backend, Backends, CliRoutes, Deployment, DeploymentBuilder, Dispatcher,
    ExitStatus, Extensions, FirstArgSelector, FromEnv, FutureResult, Guest, GuestArtifact,
    GuestEntry, GuestId, GuestRoutes, GuestSelector, HasDispatcher, HasExtensions, HasLimits,
    HasMounts, HasTable, Host, HostCtx, HttpBorrow, HttpCtx, HttpRoutes, LinkClient, Location,
    LogMode, Manifest, Mode, Mount, MountRegistry, NoOptions, PatternRoutes, Precompiled, Provides,
    Proxy, Registry, ResolvedPreopen, Routes, Runtime, RuntimeOptions, Server, SourceSpec,
    StoreBase, StoreConfig, StoreCtx, StoreView, Telemetry, Transport, TransportKind,
    TriggerRouter, WasmOnly, WeakRuntime, Wiring, WrpcState, as_command_chain, get_cloned,
    host_error, serve_links, sha256_digest, telemetry, wasi_view,
};
#[cfg(feature = "cli")]
pub use omnia_core::{Cli, Command, Parser};
#[doc(hidden)]
pub use omnia_core::{
    MainOptions, ManifestSource, WrpcCtxView, WrpcView, anyhow, futures, main, pastey, run,
    run_precompiled, run_with, tokio, wasmtime, wasmtime_wasi,
};
pub use omnia_host_macros::runtime;
pub use omnia_plugin::{
    ContentStore, LoadError, NoStore, Origin, PathMounts, PathSource, Plugin, PluginLoader,
    Plugins, RegistryClient, RegistrySource, ReleaseStore, WasiPlugins,
};
