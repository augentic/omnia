#![doc = include_str!("../README.md")]
#![cfg(not(target_arch = "wasm32"))]

mod command;
#[cfg(feature = "jit")]
mod compile;
mod deployment;
mod dispatch;
mod manifest;
mod options;
mod registry;
mod routing;
mod runtime;
mod scaffold;
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
pub use omnia_host_macros::runtime;
#[doc(hidden)]
pub use wrpc_wasmtime::{WrpcCtxView, WrpcView};
#[doc(hidden)]
pub use {anyhow, futures, tokio, wasmtime, wasmtime_wasi};

#[cfg(feature = "jit")]
pub use self::compile::compile;
pub use self::deployment::{Deployment, DeploymentBuilder};
pub use self::dispatch::{HostDispatch, serve_links};
pub use self::options::RuntimeOptions;
pub use self::registry::{Guest, GuestId, Registry};
pub use self::routing::{CliRoutes, HttpRoutes, Resolver, Routes, TopicRoutes, TriggerRouter};
pub use self::runtime::{ExitStatus, Runtime, RuntimeHooks};
#[doc(hidden)]
pub use self::runtime::{main, run};
pub use self::selector::{FirstArgSelector, GuestSelector};
pub use self::store::{HasHttp, StoreBase, StoreBaseBuilder, StoreCtx};
#[doc(hidden)]
pub use self::store::{Set, Unset};
pub use self::telemetry::{Telemetry, resource};
#[doc(hidden)]
pub use self::traits::assert_hosts;
pub use self::traits::{Backend, Backends, FromEnv, FutureResult, HasLimits, Host, Server};
pub use self::transport::{LinkClient, WrpcState};
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
