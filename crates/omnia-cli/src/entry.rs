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

// `plan` is pure over argv and `OMNIA_MANIFEST`, so precedence is testable without a binary
#[cfg(test)]
mod tests {
    use super::*;

    fn argv(args: &[&str]) -> Vec<OsString> {
        args.iter().map(OsString::from).collect()
    }

    fn fatal(error: PlanError) -> String {
        match error {
            PlanError::Fatal(error) => format!("{error:#}"),
            PlanError::Usage(error) => panic!("expected a fatal error, got usage: {error}"),
        }
    }

    #[test]
    fn manifest_beats_positional() {
        let long =
            plan(argv(&["bin", "run", "guest.wasm", "--manifest", "omnia.toml"]), None, true)
                .unwrap_or_else(|error| panic!("{}", fatal(error)));
        assert_eq!(long.source, RunSource::Manifest(PathBuf::from("omnia.toml")));

        let short = plan(argv(&["bin", "run", "-m", "omnia.toml"]), None, false)
            .unwrap_or_else(|error| panic!("{}", fatal(error)));
        assert_eq!(short.source, RunSource::Manifest(PathBuf::from("omnia.toml")));
    }

    #[test]
    fn omnia_manifest_env() {
        let plan =
            plan(argv(&["bin", "run", "guest.wasm"]), Some(OsString::from("from_env.toml")), false)
                .unwrap_or_else(|error| panic!("{}", fatal(error)));
        assert_eq!(plan.source, RunSource::Manifest(PathBuf::from("from_env.toml")));
    }

    #[test]
    fn positional_beats_compiled() {
        let plan = plan(argv(&["bin", "run", "guest.wasm"]), None, true)
            .unwrap_or_else(|error| panic!("{}", fatal(error)));
        assert_eq!(plan.source, RunSource::Wasm(PathBuf::from("guest.wasm")));
    }

    #[test]
    fn compiled_source() {
        let plan = plan(argv(&["bin", "run"]), None, true)
            .unwrap_or_else(|error| panic!("{}", fatal(error)));
        assert_eq!(plan.source, RunSource::CompiledIn);
    }

    #[test]
    fn no_source_fails() {
        let error = plan(argv(&["bin", "run"]), None, false)
            .expect_err("a sourceless deployment must fail");
        assert!(fatal(error).contains("no guest specified"));
    }

    #[test]
    fn command_mode() {
        let plan = plan(argv(&["bin", "run", "guest.wasm"]), None, false)
            .unwrap_or_else(|error| panic!("{}", fatal(error)));
        assert_eq!(plan.source, RunSource::Wasm(PathBuf::from("guest.wasm")));
    }

    #[test]
    fn usage_error() {
        let error = plan(argv(&["bin", "bogus"]), None, false)
            .expect_err("an unknown subcommand is a usage error");
        assert!(matches!(error, PlanError::Usage(_)));
    }

    // The flags are global: they parse before and after `run`, and every
    // repetition counts.
    #[test]
    fn run_verbose() {
        let before = plan(argv(&["bin", "-vv", "run", "guest.wasm"]), None, false)
            .unwrap_or_else(|error| panic!("{}", fatal(error)));
        assert_eq!((before.verbose, before.quiet), (2, 0));

        let after = plan(argv(&["bin", "run", "guest.wasm", "--verbose", "-v"]), None, false)
            .unwrap_or_else(|error| panic!("{}", fatal(error)));
        assert_eq!((after.verbose, after.quiet), (2, 0));
    }

    #[test]
    fn run_quiet() {
        let plan = plan(argv(&["bin", "run", "-qq", "guest.wasm"]), None, false)
            .unwrap_or_else(|error| panic!("{}", fatal(error)));
        assert_eq!((plan.verbose, plan.quiet), (0, 2));
    }

    #[test]
    fn run_verbose_quiet_conflict() {
        let error = plan(argv(&["bin", "run", "-v", "-q", "guest.wasm"]), None, false)
            .expect_err("`-v` with `-q` is a usage error");
        assert!(matches!(error, PlanError::Usage(_)));
    }

    // Past `--` the flags belong to the guest.
    #[test]
    fn run_guest_flags() {
        let plan = plan(argv(&["bin", "run", "guest.wasm", "--", "-v", "-q"]), None, false)
            .unwrap_or_else(|error| panic!("{}", fatal(error)));
        assert_eq!((plan.verbose, plan.quiet), (0, 0));
        assert_eq!(plan.args, ["-v", "-q"]);
    }
}
