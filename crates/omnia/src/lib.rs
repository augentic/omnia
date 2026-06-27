#![doc = include_str!("../README.md")]
#![cfg(not(target_arch = "wasm32"))]

#[cfg(feature = "jit")]
mod compile;
mod create;
mod dispatch;
mod manifest;
mod options;
mod registry;
mod routing;
mod runtime;
mod selector;
mod source;
mod store;
mod telemetry;
mod traits;
mod transport;
mod working_tree;

use std::path::PathBuf;

pub use clap::Parser;
use clap::Subcommand;
// The `Runtime` derive macro and the `Runtime` trait (re-exported below from
// `traits`) share a name but live in different namespaces, like `Clone`.
pub use omnia_host_macros::{Runtime, StoreContext, runtime};
// Macro-support re-exports: named by `runtime!`/`StoreContext`-generated code via
// `::omnia::…`, not part of the documented public surface.
#[doc(hidden)]
pub use wrpc_wasmtime::{WrpcCtxView, WrpcView};
#[doc(hidden)]
pub use {anyhow, futures, tokio, wasmtime, wasmtime_wasi};

// Curated public surface: only what a host server, a hand-written runtime, or the
// `runtime!` macro needs. Everything else (lifecycle helpers, dispatch, manifest,
// source, routing strategy, transport carriers) is `pub` inside a private module
// and simply not re-exported here.
#[cfg(feature = "jit")]
pub use self::compile::compile;
pub use self::create::{Compiled, RegistryBuilder};
pub use self::dispatch::{HostDispatch, serve_links};
pub use self::options::RuntimeOptions;
pub use self::registry::{Guest, GuestId, Registry};
pub use self::routing::{CliRoutes, HttpRoutes, Resolver, Routes, TopicRoutes, TriggerRouter};
pub use self::runtime::{ExitStatus, serve};
pub use self::selector::{FirstArgSelector, GuestSelector};
// Type-state markers naming the `StoreBaseBuilder` member states; users chain the
// setters and never name these directly, so they are hidden from the docs.
#[doc(hidden)]
pub use self::store::{Set, Unset};
pub use self::store::{StoreBase, StoreBaseBuilder};
pub use self::telemetry::{Telemetry, resource};
// Macro-support: named by `runtime!`-generated code via `::omnia::…` to guard
// host co-listing at compile time; not part of the documented public surface.
#[doc(hidden)]
pub use self::traits::assert_hosts;
pub use self::traits::{
    Backend, FromEnv, FutureResult, HasLimits, Host, HostKind, Runtime, Server,
};
pub use self::transport::{LinkClient, WrpcState};
// The working-tree registry (RFC-55): `WorkingTreeRegistry` is threaded into
// every store and read by the floor; `WorkingTreeEntry` exposes the two faces
// (cap-std `Dir` + absolute path) the floor resolves a lent descriptor to.
// `ResolvedPreopen` is the mount a registry is built from (a manifest `[[mount]]`
// or an alternate runtime assembling preopens programmatically).
pub use self::working_tree::{ResolvedPreopen, WorkingTreeEntry, WorkingTreeRegistry};

/// Command line interface for omnia.
#[derive(Parser, PartialEq, Eq)]
pub struct Cli {
    /// The command to execute.
    #[command(subcommand)]
    pub command: Command,
}

/// Subcommands for the omnia CLI.
#[derive(Subcommand, PartialEq, Eq)]
pub enum Command {
    /// Run a guest (single-file shorthand) or a manifest-driven deployment.
    Run {
        /// The path to the wasm file to run. The file can either be a
        /// serialized (pre-compiled) wasmtime `Component` or standard
        /// WASI component. Optional when `--config` (or `OMNIA_CONFIG`) names a
        /// deployment manifest instead.
        wasm: Option<PathBuf>,

        /// Path to a deployment manifest (`omnia.toml`) describing a multi-guest
        /// deployment. Falls back to the `OMNIA_CONFIG` environment variable.
        #[arg(short, long)]
        config: Option<PathBuf>,

        /// Arguments forwarded to the guest as its argv (everything after
        /// `--`). Empty for a long-lived server; a `wasi:cli` command reads
        /// them as `wasi:cli/environment`'s `get-arguments`. `args[0]` is the
        /// program name, which the floor supplies.
        #[arg(last = true)]
        args: Vec<String>,
    },
    /// Compile the specified wasm32-wasip2 component.
    #[cfg(feature = "jit")]
    Compile {
        /// The path to the wasm file to compile.
        wasm: PathBuf,

        /// An optional output directory. If not set, the compiled component
        /// will be written to the same location as the input file.
        #[arg(short, long)]
        output: Option<PathBuf>,
    },
}
