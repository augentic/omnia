#![doc = include_str!("../README.md")]

mod cli;
mod entry;

use std::env;
use std::process::ExitCode;

pub use clap::Parser;
use omnia_core::{Backends, MainOptions, Wiring};

pub use self::cli::{Cli, Command};

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
    let plan = match entry::plan(options, env::args_os(), env::var_os("OMNIA_CONFIG")) {
        Ok(plan) => plan,
        Err(entry::PlanError::Usage(error)) => error.exit(),
        Err(entry::PlanError::Fatal(error)) => {
            eprintln!("{error:#}");
            return ExitCode::FAILURE;
        }
    };
    omnia_core::drive_main::<B, H>(plan.into_builder()).await
}
