//! Generated `main`: direct-command planning and the standard grammar.

mod direct;
mod verbosity;

use std::env;
use std::ffi::OsString;
use std::process::ExitCode;

use anyhow::anyhow;
use omnia_cli::{CompilePlan, Plan, PlanError, RunSource};

pub use self::direct::{MainOptions, ManifestSource};
use self::verbosity::Levels;
use crate::{ConnectAll, DeploymentBuilder, Manifest, Mode, Mount, Wiring};

/// Entry point for generated `main` functions.
///
/// `options` carries the deployment the `runtime!` macro compiled in: mode
/// and manifest source. Command mode with a compiled-in deployment is a
/// direct command: argv passes to the command guest verbatim. Every other
/// shape takes the standard grammar:
///
/// - `run [wasm] [--manifest] -- args…` runs a deployment.
/// - `compile <wasm>` compiles a component under the compile-affecting
///   settings of the [`RuntimeOptions`](crate::RuntimeOptions) the
///   environment sets. A runtime built without omnia's `jit` feature refuses
///   it.
#[doc(hidden)]
pub async fn main<B, H>(options: MainOptions) -> ExitCode
where
    B: ConnectAll,
    H: Wiring<B>,
{
    match plan(options) {
        Ok(Planned::Run(plan)) => crate::lifecycle::drive_main::<B, H>(plan.into_builder()).await,
        Ok(Planned::Compile(plan)) => match compile(plan) {
            Ok(()) => ExitCode::SUCCESS,
            Err(error) => crate::lifecycle::report(&error),
        },
        Err(error) => crate::lifecycle::report(&error),
    }
}

fn plan(options: MainOptions) -> anyhow::Result<Planned> {
    if options.is_direct() {
        return Ok(Planned::Run(Box::new(direct::plan_direct(options, env::args_os())?)));
    }
    match materialize(options, env::args_os(), env::var_os("OMNIA_MANIFEST")) {
        Ok(planned) => Ok(planned),
        Err(PlanError::Usage(error)) => error.exit(),
        Err(PlanError::Fatal(error)) => Err(error),
    }
}

enum Planned {
    Run(Box<EntryPlan>),
    Compile(CompilePlan),
}

cfg_select! {
    feature = "jit" => {
        // the environment's settings, so the artifact loads into a runtime
        // started with the same environment
        fn compile(plan: CompilePlan) -> anyhow::Result<()> {
            let options = crate::RuntimeOptions::load_env()?.compile_options();
            crate::compile::compile(&plan.wasm, plan.output, plan.target.as_deref(), &options)
        }
    }
    _ => {
        fn compile(_: CompilePlan) -> anyhow::Result<()> {
            anyhow::bail!(
                "`compile` needs omnia's `jit` feature, which this runtime was built without"
            )
        }
    }
}

// a malformed `--env` is refused before any manifest loads, so the error
// names the flag rather than the `[env]` table it joins
fn materialize(
    options: MainOptions, argv: impl IntoIterator<Item = OsString>,
    omnia_manifest: Option<OsString>,
) -> Result<Planned, PlanError> {
    let (mode, compiled_in, program_name) = options.into_parts();
    let plan = match omnia_cli::plan(argv, omnia_manifest, compiled_in.is_some())? {
        Plan::Run(plan) => plan,
        Plan::Compile(plan) => return Ok(Planned::Compile(plan)),
    };
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
    Ok(Planned::Run(Box::new(EntryPlan {
        mode,
        manifest: Some(manifest.mounts(mounts).inherit(plan.env)),
        program_name,
        args: plan.args,
        levels: verbosity::levels(mode, plan.verbose, plan.quiet),
    })))
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
    use std::path::PathBuf;

    use super::*;
    use crate::GuestEntry;

    fn argv(args: &[&str]) -> Vec<OsString> {
        args.iter().map(OsString::from).collect()
    }

    fn server(manifest: Manifest) -> MainOptions {
        MainOptions::new(Mode::Server).manifest(ManifestSource::Inline(manifest))
    }

    // `PlanError` is not `Debug`, so each outcome is unpacked by hand
    fn planned(manifest: Manifest, args: &[&str]) -> Planned {
        match materialize(server(manifest), argv(args), None) {
            Ok(planned) => planned,
            Err(PlanError::Usage(error)) => panic!("expected a plan, got usage: {error}"),
            Err(PlanError::Fatal(error)) => panic!("expected a plan, got fatal: {error:#}"),
        }
    }

    #[test]
    fn run_env() {
        let manifest = Manifest::new().guest(GuestEntry::new("app", "app.wasm")).inherit(["HOME"]);
        let Planned::Run(plan) = planned(manifest, &["bin", "run", "--env", "ORDERS_*"]) else {
            panic!("`run` plans a run");
        };
        let manifest = plan.manifest.expect("the plan carries the compiled-in manifest");
        assert_eq!(manifest.env.inherit, ["HOME", "ORDERS_*"]);
    }

    // The compiled-in manifest is no source for `compile`, which names its
    // component outright.
    #[test]
    fn compile() {
        let manifest = Manifest::new().guest(GuestEntry::new("app", "app.wasm"));
        let args =
            ["bin", "compile", "guest.wasm", "-o", "guest.bin", "-t", "aarch64-unknown-linux-gnu"];
        let Planned::Compile(plan) = planned(manifest, &args) else {
            panic!("`compile` plans a compile");
        };
        let expected = CompilePlan {
            wasm: PathBuf::from("guest.wasm"),
            output: Some(PathBuf::from("guest.bin")),
            target: Some("aarch64-unknown-linux-gnu".to_owned()),
        };
        assert_eq!(plan, expected);
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
