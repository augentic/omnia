//! Command-line interface for omnia.

use std::path::PathBuf;

use clap::{Parser, Subcommand};

use crate::Mount;

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

        /// Preopen a host directory into the guest sandbox (repeatable).
        /// Format: `path=<host-path>[,name=<guest-name>][,writable]`; `name`
        /// defaults to `.`. Layered on top of the manifest's mounts when
        /// `--config` is also given; a matching guest-visible name overrides the
        /// manifest mount (last-wins).
        #[arg(long = "mount")]
        mounts: Vec<Mount>,

        /// Host-mediated interface to dispatch on the guest's behalf
        /// (repeatable). Unioned with the manifest's per-guest `link` lists
        /// when `--config` is also given.
        #[arg(long = "link")]
        links: Vec<String>,

        /// Arguments forwarded to the guest as its argv (everything after
        /// `--`). Empty for a long-lived server; a `wasi:cli` command reads
        /// them as `wasi:cli/environment`'s `get-arguments`. `args[0]` is the
        /// program name, which the runtime core supplies.
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
