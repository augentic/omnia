#![doc = include_str!("../README.md")]
#![cfg(not(target_arch = "wasm32"))]

// The embedder facade: the runtime spine (`omnia-core`), the plugins
// capability (`omnia-plugin`), and the `runtime!` macro, re-exported under
// one root. The `runtime!` macro emits `omnia::…` paths, so every name it
// references must stay reachable from here.
//
// `#[doc(inline)]` matters: rustdoc renders a cross-crate `pub use` as a bare
// re-export line pointing into the source crate, so without it every item
// page (and every path readers copy) would spell `omnia_core::…`. Inlining
// keeps the documented surface at `omnia::…`, the only path embedders use.

// `anyhow` is the error vocabulary of `Backend`, `Wiring`, and the generated
// runtime module; `futures` supplies the `BoxFuture` in the plugin store and
// acquirer seams (`ContentStore`, `ReleaseStore`, `PathSource`,
// `RegistrySource`). Both are part of the facade's public signatures, so
// embedders reach them from here without a direct dependency of their own.
pub use anyhow;
pub use futures;
#[cfg(feature = "jit")]
#[doc(inline)]
pub use omnia_core::compile;
#[doc(inline)]
pub use omnia_core::{
    AdmitError, Backend, Backends, CliRoutes, Deployment, DeploymentBuilder, Dispatcher,
    ExitStatus, Extensions, FirstArgSelector, FromEnv, FutureResult, Guest, GuestArtifact,
    GuestEntry, GuestId, GuestRoutes, GuestSelector, HasDispatcher, HasExtensions, HasLimits,
    HasMounts, HasTable, Host, HostCtx, HttpBorrow, HttpCtx, HttpRoutes, LinkClient, LogMode,
    Manifest, Mode, Mount, MountRegistry, NoOptions, PatternRoutes, Precompiled, Provides, Proxy,
    Registry, ResolvedPreopen, Routes, Runtime, RuntimeOptions, Server, SourceSpec, StoreBase,
    StoreConfig, StoreCtx, StoreView, Telemetry, Transport, TransportKind, TriggerRouter, WasmOnly,
    WeakRuntime, Wiring, WrpcState, as_command_chain, get_cloned, host_error, serve_links,
    sha256_digest, telemetry, wasi_view,
};
#[cfg(feature = "cli")]
#[doc(inline)]
pub use omnia_core::{Cli, Command, Parser};
#[doc(hidden)]
pub use omnia_core::{
    MainOptions, ManifestSource, WrpcCtxView, WrpcView, main, pastey, run, run_precompiled, tokio,
    wasmtime, wasmtime_wasi,
};
#[doc(inline)]
pub use omnia_host_macros::runtime;
#[doc(inline)]
pub use omnia_plugin::{
    ContentStore, LoadError, Location, NoStore, PathMounts, PathSource, Plugin, PluginLoader,
    Plugins, RegistryClient, RegistrySource, ReleaseStore, WasiPlugins, WasiPluginsCtxView,
};
