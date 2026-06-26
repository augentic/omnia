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
mod traits;
mod transport;

use std::path::PathBuf;

pub use clap::Parser;
use clap::Subcommand;
pub use omnia_runtime_macro::runtime;
// Re-exported so the `runtime!` macro can generate the per-store
// `WrpcView` implementation that host-mediated dynamic linking requires.
pub use wrpc_wasmtime::{WrpcCtxView, WrpcView};
#[doc(hidden)]
pub use {anyhow, futures, tokio, wasmtime, wasmtime_wasi};

// re-export internal modules
#[cfg(feature = "jit")]
pub use self::compile::*;
pub use self::create::*;
pub use self::dispatch::*;
pub use self::manifest::*;
pub use self::options::*;
pub use self::registry::*;
pub use self::routing::*;
pub use self::runtime::*;
pub use self::selector::*;
pub use self::source::*;
pub use self::traits::*;
pub use self::transport::*;

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
