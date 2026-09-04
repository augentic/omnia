#![doc = include_str!("../README.md")]
#![cfg(not(target_arch = "wasm32"))]

// The embedder facade: the runtime spine (`omnia-core`), the plugins
// capability (`omnia-plugin`), the `run` grammar (`omnia-cli`, behind the
// `cli` feature), and the `runtime!` macro, re-exported under one root. The
// `runtime!` macro emits `omnia::…` paths, so every name it references must
// stay reachable from here.
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
#[cfg(feature = "cli")]
#[doc(hidden)]
pub use omnia_cli::main;
#[cfg(feature = "cli")]
#[doc(inline)]
pub use omnia_cli::{Cli, Command, Parser};
#[cfg(feature = "jit")]
#[doc(inline)]
pub use omnia_core::compile;
#[cfg(not(feature = "cli"))]
#[doc(hidden)]
pub use omnia_core::main;
#[doc(inline)]
pub use omnia_core::{
    AdmitError, Backend, Backends, CliRoutes, Deployment, DeploymentBuilder, Dispatcher,
    ExitStatus, Extensions, FirstArgSelector, FromEnv, FutureResult, Guest, GuestArtifact,
    GuestEntry, GuestId, GuestRoutes, GuestSelector, HasDispatcher, HasExtensions, HasLimits,
    HasMounts, HasTable, Host, HostCtx, HttpBorrow, HttpCtx, HttpRoutes, LinkClient, Location,
    LogMode, Manifest, Mode, Mount, MountRegistry, NoOptions, PatternRoutes, Provides, Proxy,
    Registry, ResolvedPreopen, Routes, Runtime, RuntimeOptions, RuntimeParts, Server, SourceSpec,
    StoreBase, StoreConfig, StoreCtx, StoreView, Telemetry, Transport, TransportKind,
    TriggerRouter, WeakRuntime, Wiring, WrpcState, as_command_chain, get_cloned, host_error,
    serve_links, sha256_digest, telemetry, wasi_view,
};
#[doc(hidden)]
pub use omnia_core::{
    MainOptions, ManifestSource, WrpcCtxView, WrpcView, pastey, run, run_with, tokio, wasmtime,
    wasmtime_wasi,
};
#[doc(inline)]
pub use omnia_host_macros::runtime;
#[doc(inline)]
pub use omnia_plugin::{
    ContentStore, LoadError, NoStore, Origin, PathMounts, PathSource, Plugin, PluginLoader,
    Plugins, RegistryClient, RegistrySource, ReleaseStore, WasiPlugins, WasiPluginsCtxView,
};
