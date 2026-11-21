//! Generated `main`: direct-command planning and the `run` grammar.

mod direct;
mod verbosity;

use std::env;
use std::ffi::OsString;
use std::process::ExitCode;

use omnia_cli::{PlanError, RunSource};

pub use self::direct::{MainOptions, ManifestSource};
use crate::{ConnectAll, DeploymentBuilder, Manifest, Mount, Wiring};

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
        Ok(builder) => Ok(builder),
        Err(PlanError::Usage(error)) => error.exit(),
        Err(PlanError::Fatal(error)) => Err(error),
    }
}

fn materialize(
    options: MainOptions, argv: impl IntoIterator<Item = OsString>,
    omnia_manifest: Option<OsString>,
) -> Result<DeploymentBuilder, PlanError> {
    let (mode, compiled_in, program_name) = options.into_parts();
    let plan = omnia_cli::plan(argv, omnia_manifest, compiled_in.is_some())?;
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
    let manifest = manifest.mounts(mounts).inherit(plan.env);
    let builder = DeploymentBuilder::new().manifest(manifest).args(plan.args).mode(mode);
    let builder = match program_name {
        Some(name) => builder.program_name(name),
        None => builder,
    };
    Ok(verbosity::levels(mode, plan.verbose, plan.quiet).apply(builder))
}
