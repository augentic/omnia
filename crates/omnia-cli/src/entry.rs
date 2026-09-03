//! Entry planning for the `run` grammar: macro-compiled [`MainOptions`] plus
//! process argv and environment resolve into a deployment builder.
//!
//! [`plan`] is pure with respect to the process — argv and `OMNIA_CONFIG` are
//! parameters — so source precedence is unit-testable without spawning a
//! binary.

use std::ffi::OsString;
use std::path::PathBuf;

use anyhow::anyhow;
use clap::Parser as _;
use omnia_core::{DeploymentBuilder, MainOptions, Manifest, Mode};

use crate::cli::{Cli, Command};

/// Why entry planning stopped before a deployment could be built.
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

/// The planner's outcome: every deployment decision, resolved.
pub struct Plan {
    mode: Mode,
    manifest: Option<Manifest>,
    args: Vec<String>,
}

impl Plan {
    /// Assemble the deployment builder this plan describes.
    pub fn into_builder(self) -> DeploymentBuilder {
        DeploymentBuilder::new().manifest(self.manifest).args(self.args).mode(self.mode)
    }
}

/// Plan the standard `run [wasm] [--config] -- args…` grammar, resolving the
/// manifest by the `--config` › `OMNIA_CONFIG` › positional wasm › compiled-in
/// ladder.
pub fn plan(
    options: MainOptions, argv: impl IntoIterator<Item = OsString>, omnia_config: Option<OsString>,
) -> Result<Plan, PlanError> {
    let (mode, manifest) = options.into_parts();
    let cli = Cli::try_parse_from(argv).map_err(PlanError::Usage)?;
    match cli.command {
        Command::Run {
            wasm,
            config,
            mounts,
            plugins,
            args,
        } => {
            let config = config.or_else(|| omnia_config.map(PathBuf::from));
            let manifest = match (config, wasm) {
                (Some(config), _) => Manifest::from_config(config)?,
                (None, Some(wasm)) => Manifest::from_wasm(wasm),
                (None, None) => match manifest {
                    Some(source) => source.into_manifest()?,
                    None => {
                        return Err(PlanError::Fatal(anyhow!(
                            "no guest specified: pass a <wasm> path, or --config <omnia.toml> (or \
                             set OMNIA_CONFIG)"
                        )));
                    }
                },
            };
            Ok(Plan {
                mode,
                manifest: Some(manifest.mounts(mounts).plugins(plugins)),
                args,
            })
        }
        #[cfg(feature = "jit")]
        Command::Compile { .. } => Err(PlanError::Fatal(anyhow!(
            "the generated `main` only supports `run`; supply a custom `main` for other subcommands"
        ))),
    }
}

// Unit tests by design: `plan` is factored pure (argv and `OMNIA_CONFIG` are
// parameters) precisely so source precedence is testable without spawning a
// binary.
#[cfg(test)]
mod tests {
    use omnia_core::{GuestEntry, ManifestSource};

    use super::*;

    fn argv(args: &[&str]) -> Vec<OsString> {
        args.iter().map(OsString::from).collect()
    }

    fn inline_source(guest: &str) -> ManifestSource {
        ManifestSource::Inline(
            Manifest::new().guest(GuestEntry::new(guest, format!("{guest}.wasm"))),
        )
    }

    fn first_guest(plan: &Plan) -> &str {
        plan.manifest.as_ref().expect("plan carries a manifest").guests[0].id.as_str()
    }

    fn temp_manifest(tag: &str, guest: &str) -> PathBuf {
        let path =
            std::env::temp_dir().join(format!("omnia_cli_{tag}_{}.toml", std::process::id()));
        std::fs::write(&path, format!("[[guest]]\nid = \"{guest}\"\nsource.path = \"./g.wasm\"\n"))
            .expect("temp manifest should write");
        path
    }

    fn fatal(error: PlanError) -> String {
        match error {
            PlanError::Fatal(error) => format!("{error:#}"),
            PlanError::Usage(error) => panic!("expected a fatal error, got usage: {error}"),
        }
    }

    #[test]
    fn config_beats_positional_wasm_and_compiled_source() {
        let path = temp_manifest("precedence", "from_config");
        let options = MainOptions::new(Mode::Server).manifest(inline_source("compiled"));
        let plan = plan(
            options,
            argv(&["bin", "run", "guest.wasm", "--config", path.to_str().unwrap()]),
            None,
        )
        .unwrap_or_else(|error| panic!("{}", fatal(error)));
        let _ = std::fs::remove_file(&path);
        assert_eq!(first_guest(&plan), "from_config");
    }

    #[test]
    fn omnia_config_env_beats_positional_wasm() {
        let path = temp_manifest("env", "from_env");
        let plan = plan(
            MainOptions::new(Mode::Server),
            argv(&["bin", "run", "guest.wasm"]),
            Some(path.clone().into_os_string()),
        )
        .unwrap_or_else(|error| panic!("{}", fatal(error)));
        let _ = std::fs::remove_file(&path);
        assert_eq!(first_guest(&plan), "from_env");
    }

    #[test]
    fn positional_wasm_beats_compiled_source() {
        let options = MainOptions::new(Mode::Server).manifest(inline_source("compiled"));
        let plan = plan(options, argv(&["bin", "run", "guest.wasm"]), None)
            .unwrap_or_else(|error| panic!("{}", fatal(error)));
        assert_eq!(first_guest(&plan), "guest");
    }

    #[test]
    fn compiled_source_is_the_fallback() {
        let options = MainOptions::new(Mode::Server).manifest(inline_source("compiled"));
        let plan = plan(options, argv(&["bin", "run"]), None)
            .unwrap_or_else(|error| panic!("{}", fatal(error)));
        assert_eq!(first_guest(&plan), "compiled");
    }

    #[test]
    fn no_source_fails() {
        let error = plan(MainOptions::new(Mode::Server), argv(&["bin", "run"]), None)
            .err()
            .expect("a sourceless deployment must fail");
        assert!(fatal(error).contains("no guest specified"));
    }

    #[test]
    fn command_mode_without_deployment_keeps_run_grammar() {
        let plan = plan(MainOptions::new(Mode::Command), argv(&["bin", "run", "guest.wasm"]), None)
            .unwrap_or_else(|error| panic!("{}", fatal(error)));
        assert_eq!(first_guest(&plan), "guest");
    }

    #[test]
    fn compiled_path_load_failure_surfaces() {
        let options = MainOptions::new(Mode::Server)
            .manifest(ManifestSource::Path(PathBuf::from("/nonexistent/omnia.toml")));
        let error = plan(options, argv(&["bin", "run"]), None)
            .err()
            .expect("a missing compiled-in manifest path must fail");
        assert!(fatal(error).contains("reading manifest"));
    }

    #[test]
    fn usage_error_is_delegated_to_clap() {
        let error = plan(MainOptions::new(Mode::Server), argv(&["bin", "bogus"]), None)
            .err()
            .expect("an unknown subcommand is a usage error");
        assert!(matches!(error, PlanError::Usage(_)));
    }
}
