#![cfg(not(target_arch = "wasm32"))]

//! # WebAssembly Initiator

#[cfg(feature = "jit")]
mod compile;
mod create;
mod traits;

use std::path::PathBuf;

pub use clap::Parser;
use clap::Subcommand;
pub use yetti_runtime_macro::runtime;
pub use {anyhow, futures, tokio, wasmtime, wasmtime_wasi};

// re-export internal modules
#[cfg(feature = "jit")]
pub use self::compile::*;
pub use self::create::*;
pub use self::traits::*;

#[derive(Parser, PartialEq, Eq)]
#[command(version, about, long_about = None)]
pub struct Cli {
    /// The command to execute.
    #[command(subcommand)]
    pub command: Command,
}

#[derive(Subcommand, PartialEq, Eq)]
pub enum Command {
    /// Run the specified wasm guest.
    Run {
        /// The path to the wasm file to run. The file can either be a
        /// serialized (pre-compiled) wasmtime `Component` or standard
        /// WASI component
        wasm: PathBuf,
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
