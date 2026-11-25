//! Entry planning for the standard grammar: process argv and environment
//! resolve into a [`Plan`] over paths and strings.
//!
//! [`plan`] is pure with respect to the process — argv and `OMNIA_MANIFEST`
//! are parameters — so source precedence is testable without spawning a
//! binary.

use std::ffi::OsString;
use std::path::PathBuf;

use anyhow::anyhow;
use clap::Parser as _;

use crate::cli::{Cli, Command, MountArg};

/// Plan the standard grammar: `run [wasm] [--manifest] -- args…` or
/// `compile <wasm> [--output] [--target]`.
///
/// A `run` resolves its source by the `--manifest` › `OMNIA_MANIFEST` ›
/// positional wasm › compiled-in ladder. `has_compiled_in` is whether the
/// generated `main` compiled a manifest in; this function does not load it.
/// A `compile` names its component outright, so the ladder does not apply.
///
/// # Errors
///
/// Returns [`PlanError::Usage`] when clap rejects argv, or [`PlanError::Fatal`]
/// when a `run` has no source available.
pub fn plan(
    argv: impl IntoIterator<Item = OsString>, omnia_manifest: Option<OsString>,
    has_compiled_in: bool,
) -> Result<Plan, PlanError> {
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
            env,
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
            Ok(Plan::Run(RunPlan {
                source,
                mounts,
                env,
                verbose,
                quiet,
                args,
            }))
        }
        Command::Compile { wasm, output, target } => {
            Ok(Plan::Compile(CompilePlan { wasm, output, target }))
        }
    }
}

/// The planner's outcome: the subcommand argv selects, with its arguments resolved.
#[derive(Debug, PartialEq, Eq)]
pub enum Plan {
    /// Run a deployment.
    Run(RunPlan),
    /// Compile a component ahead of time.
    Compile(CompilePlan),
}

/// A `run` plan: source, CLI mounts and environment, verbosity, and the
/// command's arguments.
#[derive(Debug, PartialEq, Eq)]
pub struct RunPlan {
    /// Which source the precedence ladder selected.
    pub source: RunSource,
    /// `--mount` arguments, in argv order.
    pub mounts: Vec<MountArg>,
    /// `--env` names, in argv order: the host variables every guest sees.
    pub env: Vec<String>,
    /// How many `-v` flags were given; each raises the process tracing level
    /// one step.
    pub verbose: u8,
    /// How many `-q` flags were given; each lowers the process tracing level
    /// one step. Never non-zero together with `verbose`: clap refuses the
    /// pair.
    pub quiet: u8,
    /// The arguments for the command guest's run (everything after `--`).
    pub args: Vec<String>,
}

/// A `compile` plan: the component, where its artifact goes, and the target.
#[derive(Debug, PartialEq, Eq)]
pub struct CompilePlan {
    /// The `wasm32-wasip3` component to compile.
    pub wasm: PathBuf,
    /// Where to write the artifact: a file path, or an existing directory
    /// the artifact lands in as `<input stem>.bin`. Unset, the artifact is
    /// written to stdout.
    pub output: Option<PathBuf>,
    /// The target triple the artifact runs on; the host's when unset.
    pub target: Option<String>,
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

/// Why entry planning stopped before a plan could be produced.
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
