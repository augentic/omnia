#![doc = include_str!("../README.md")]

mod cli;
mod entry;

use std::env;
use std::ffi::OsString;
use std::process::ExitCode;

pub use clap::Parser;
use omnia_core::{Backends, DeploymentBuilder, MainOptions, Manifest, Mount, Wiring};

pub use self::cli::{Cli, Command, MountArg};
use self::entry::{PlanError, RunSource};

/// Entry point for generated `main` functions built with the `cli` feature.
///
/// A direct command (command mode with a compiled-in manifest) is handed to
/// `omnia-core` untouched; every other shape parses the standard
/// `run [wasm] [--config] -- args…` grammar.
#[doc(hidden)]
pub async fn main<B, H>(options: MainOptions) -> ExitCode
where
    B: Backends,
    H: Wiring<B>,
{
    if options.is_direct() {
        return omnia_core::main::<B, H>(options).await;
    }
    let builder = match materialize(options, env::args_os(), env::var_os("OMNIA_CONFIG")) {
        Ok(builder) => builder,
        Err(PlanError::Usage(error)) => error.exit(),
        Err(PlanError::Fatal(error)) => {
            eprintln!("{error:#}");
            return ExitCode::FAILURE;
        }
    };
    omnia_core::drive_main::<B, H>(builder).await
}

fn materialize(
    options: MainOptions, argv: impl IntoIterator<Item = OsString>, omnia_config: Option<OsString>,
) -> Result<DeploymentBuilder, PlanError> {
    let (mode, compiled_in) = options.into_parts();
    let plan = entry::plan(argv, omnia_config, compiled_in.is_some())?;
    let manifest = match plan.source {
        RunSource::Config(path) => Manifest::from_config(path)?,
        RunSource::Wasm(path) => Manifest::from_wasm(path),
        RunSource::CompiledIn => compiled_in.expect("planner checked").into_manifest()?,
    };
    let mounts = plan.mounts.into_iter().map(Mount::from);
    Ok(DeploymentBuilder::new()
        .manifest(manifest.mounts(mounts).plugins(plan.plugins))
        .args(plan.args)
        .mode(mode))
}

impl From<MountArg> for Mount {
    fn from(arg: MountArg) -> Self {
        Self {
            name: arg.name,
            path: arg.host_path,
            writable: arg.writable,
        }
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use omnia_core::{MainOptions, ManifestSource, Mode};

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
    fn compiled_path_load_failure_surfaces() {
        let options = MainOptions::new(Mode::Server)
            .manifest(ManifestSource::Path(PathBuf::from("/nonexistent/omnia.toml")));
        let error = materialize(options, argv(&["bin", "run"]), None)
            .expect_err("a missing compiled-in manifest path must fail");
        assert!(fatal(error).contains("reading manifest"));
    }
}
