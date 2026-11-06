//! Entry planning for the `run` grammar: process argv and environment resolve
//! into a [`RunPlan`] over paths and strings.
//!
//! [`plan`] is pure with respect to the process — argv and `OMNIA_MANIFEST`
//! are parameters — so source precedence is unit-testable without spawning a
//! binary.

use std::ffi::OsString;
use std::path::PathBuf;

use anyhow::anyhow;
use clap::Parser as _;

use crate::cli::{Cli, Command, MountArg};

/// Why entry planning stopped before a run plan could be produced.
pub enum PlanError {
    /// A clap-level outcome (usage error, `--help`, `--version`); the caller
    /// delegates to [`clap::Error::exit`] so stream and exit code match the
    /// standard CLI behavior.
    Usage(clap::Error),
    /// A startup failure reported on stderr.
    Fatal(anyhow::Error),
}

impl From<anyhow::Error> for PlanError {
    fn from(error: anyhow::Error) -> Self {
        Self::Fatal(error)
    }
}

/// Where the deployment comes from — the `--manifest` › `OMNIA_MANIFEST` ›
/// `<wasm>` › compiled-in ladder, decided over plain data.
#[derive(Debug, PartialEq, Eq)]
pub enum RunSource {
    /// A `--manifest` / `OMNIA_MANIFEST` manifest path.
    Manifest(PathBuf),
    /// A positional wasm path.
    Wasm(PathBuf),
    /// The generated `main`'s compiled-in manifest, not loaded here.
    CompiledIn,
}

/// The planner's outcome: source, CLI mounts, verbosity, and guest argv.
#[derive(Debug, PartialEq, Eq)]
pub struct RunPlan {
    /// Which source the precedence ladder selected.
    pub source: RunSource,
    /// `--mount` arguments, in argv order.
    pub mounts: Vec<MountArg>,
    /// How many `-v` flags were given; each raises the process tracing level
    /// one step.
    pub verbose: u8,
    /// How many `-q` flags were given; each lowers the process tracing level
    /// one step. Never non-zero together with `verbose`: clap refuses the
    /// pair.
    pub quiet: u8,
    /// Arguments forwarded to the guest as its argv (everything after `--`).
    pub args: Vec<String>,
}

/// Plan the standard `run [wasm] [--manifest] -- args…` grammar, resolving
/// the source by the `--manifest` › `OMNIA_MANIFEST` › positional wasm ›
/// compiled-in ladder.
///
/// `has_compiled_in` is whether the generated `main` compiled a manifest in;
/// this function does not load it.
///
/// # Errors
///
/// Returns [`PlanError::Usage`] when clap rejects argv, or [`PlanError::Fatal`]
/// when no source is available or the subcommand is not `run`.
pub fn plan(
    argv: impl IntoIterator<Item = OsString>, omnia_manifest: Option<OsString>,
    has_compiled_in: bool,
) -> Result<RunPlan, PlanError> {
    let Cli {
        command,
        verbose,
        quiet,
    } = Cli::try_parse_from(argv).map_err(PlanError::Usage)?;
    match command {
        Command::Run {
            wasm,
            manifest,
            mounts,
            args,
        } => {
            let manifest = manifest.or_else(|| omnia_manifest.map(PathBuf::from));
            let source = match (manifest, wasm) {
                (Some(manifest), _) => RunSource::Manifest(manifest),
                (None, Some(wasm)) => RunSource::Wasm(wasm),
                (None, None) if has_compiled_in => RunSource::CompiledIn,
                (None, None) => {
                    return Err(PlanError::Fatal(anyhow!(
                        "no guest specified: pass a <wasm> path, or --manifest <omnia.toml> (or \
                         set OMNIA_MANIFEST)"
                    )));
                }
            };
            Ok(RunPlan {
                source,
                mounts,
                verbose,
                quiet,
                args,
            })
        }
        #[cfg(feature = "jit")]
        Command::Compile { .. } => Err(PlanError::Fatal(anyhow!(
            "the generated `main` only supports `run`; supply a custom `main` for other subcommands"
        ))),
    }
}
