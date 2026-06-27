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
pub use self::routing::{HttpRoutes, Resolver, Routes, TopicRoutes, TriggerRouter};
pub use self::runtime::serve;
pub use self::selector::{FirstArgSelector, GuestSelector};
pub use self::store::StoreBase;
pub use self::telemetry::{Telemetry, resource};
pub use self::traits::{Backend, FromEnv, FutureResult, HasLimits, Host, Runtime, Server};
pub use self::transport::{LinkClient, WrpcState};

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

        /// Path to a deployment manifest (`omni.toml`) describing a multi-guest
        /// deployment. Falls back to the `OMNIA_CONFIG` environment variable.
        #[arg(short, long)]
        config: Option<PathBuf>,
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
