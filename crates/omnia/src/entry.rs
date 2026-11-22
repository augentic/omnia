//! Generated `main`: direct-command planning and the `run` grammar.

mod direct;
mod verbosity;

use std::env;
use std::ffi::OsString;
use std::process::ExitCode;

use anyhow::anyhow;
use omnia_cli::{PlanError, RunSource};

pub use self::direct::{MainOptions, ManifestSource};
use self::verbosity::Levels;
use crate::{ConnectAll, DeploymentBuilder, Manifest, Mode, Mount, Wiring};

/// Entry point for generated `main` functions.
///
/// `options` carries the deployment the `runtime!` macro compiled in: mode
/// and manifest source. Command mode with a compiled-in deployment is a
/// direct command: argv passes to the guest verbatim. Every other shape
/// takes the standard `run [wasm] [--manifest] -- args…` grammar.
#[doc(hidden)]
pub async fn main<B, H>(options: MainOptions) -> ExitCode
where
    B: ConnectAll,
    H: Wiring<B>,
{
    match builder(options) {
        Ok(builder) => crate::lifecycle::drive_main::<B, H>(builder).await,
        Err(error) => crate::lifecycle::report(&error),
    }
}

fn builder(options: MainOptions) -> anyhow::Result<DeploymentBuilder> {
    if options.is_direct() {
        return Ok(direct::plan_direct(options, env::args_os())?.into_builder());
    }
    match materialize(options, env::args_os(), env::var_os("OMNIA_MANIFEST")) {
        Ok(plan) => Ok(plan.into_builder()),
        Err(PlanError::Usage(error)) => error.exit(),
        Err(PlanError::Fatal(error)) => Err(error),
    }
}

// a malformed `--env` is refused before any manifest loads, so the error
// names the flag rather than the `[env]` table it joins
fn materialize(
    options: MainOptions, argv: impl IntoIterator<Item = OsString>,
    omnia_manifest: Option<OsString>,
) -> Result<EntryPlan, PlanError> {
    let (mode, compiled_in, program_name) = options.into_parts();
    let plan = omnia_cli::plan(argv, omnia_manifest, compiled_in.is_some())?;
    for entry in &plan.env {
        omnia_core::env::pattern(entry).map_err(|error| {
            anyhow!("`--env` {error}; quote a pattern so the shell does not expand it")
        })?;
    }
    let manifest = match plan.source {
        RunSource::Manifest(path) => Manifest::load(path)?,
        RunSource::Wasm(path) => Manifest::from_wasm(path)?,
        RunSource::CompiledIn => compiled_in.expect("planner checked").into_manifest()?,
    };
    let mounts = plan.mounts.into_iter().map(|arg| Mount {
        name: arg.name,
        path: arg.host_path,
        writable: arg.writable,
    });
    Ok(EntryPlan {
        mode,
        manifest: Some(manifest.mounts(mounts).inherit(plan.env)),
        program_name,
        args: plan.args,
        levels: verbosity::levels(mode, plan.verbose, plan.quiet),
    })
}

/// The planner's outcome: every deployment decision, resolved.
pub struct EntryPlan {
    mode: Mode,
    manifest: Option<Manifest>,
    program_name: Option<String>,
    args: Vec<String>,
    levels: Levels,
}

impl EntryPlan {
    /// Assemble the deployment builder this plan describes.
    pub fn into_builder(self) -> DeploymentBuilder {
        let builder =
            DeploymentBuilder::new().manifest(self.manifest).args(self.args).mode(self.mode);
        let builder = match self.program_name {
            Some(name) => builder.program_name(name),
            None => builder,
        };
        self.levels.apply(builder)
    }
}

// Unit tests by design: `materialize` takes argv and `OMNIA_MANIFEST` as
// parameters precisely so the `run` grammar's policy is testable without
// spawning a binary.
#[cfg(test)]
mod tests {
    use super::*;
    use crate::GuestEntry;

    fn argv(args: &[&str]) -> Vec<OsString> {
        args.iter().map(OsString::from).collect()
    }

    fn server(manifest: Manifest) -> MainOptions {
        MainOptions::new(Mode::Server).manifest(ManifestSource::Inline(manifest))
    }

    #[test]
    fn run_env() {
        let manifest = Manifest::new().guest(GuestEntry::new("app", "app.wasm")).inherit(["HOME"]);
        let plan =
            match materialize(server(manifest), argv(&["bin", "run", "--env", "ORDERS_*"]), None) {
                Ok(plan) => plan,
                Err(PlanError::Usage(error)) => panic!("expected a plan, got usage: {error}"),
                Err(PlanError::Fatal(error)) => panic!("expected a plan, got fatal: {error:#}"),
            };
        let manifest = plan.manifest.expect("the plan carries the compiled-in manifest");
        assert_eq!(manifest.env.inherit, ["HOME", "ORDERS_*"]);
    }

    // `Cargo.toml` is what an unquoted `*` becomes in a crate's directory.
    #[test]
    fn malformed_env() {
        for entry in ["FOO=bar", "Cargo.toml"] {
            let manifest = Manifest::new().guest(GuestEntry::new("app", "app.wasm"));
            match materialize(server(manifest), argv(&["bin", "run", "--env", entry]), None) {
                Err(PlanError::Fatal(error)) => {
                    let error = format!("{error:#}");
                    assert!(error.contains(&format!("`--env` `{entry}`")), "{error}");
                }
                Err(PlanError::Usage(error)) => {
                    panic!("expected a fatal error, got usage: {error}")
                }
                Ok(_) => panic!("`--env {entry}` is not a variable pattern"),
            }
        }
    }
}
