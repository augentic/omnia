//! Keeps the `omnia:*` WIT in step with what is built from it.
//!
//! - The copies each crate and example keeps under its `wit/deps`.
//! - The `omnia-sdk` guest bindings under `crates/sdk/src/bindings`.
//! - The releases the wasm-pkg registry holds of each published package.
//!
//! Each command runs as `cargo xtask <command>`, from anywhere in the workspace.

mod package;
mod registry;
mod sdk;
mod workspace;

use std::path::{self, Path, PathBuf};
use std::process::ExitCode;
use std::{env, fs};

use anyhow::{Context, Result, ensure};
use clap::{Parser, Subcommand};

use crate::registry::{Registry, Release, State};
use crate::sdk::Sdk;
use crate::workspace::Workspace;

#[derive(Parser)]
#[command(about)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Rewrite every `wit/deps` copy from the package that owns it.
    Sync,
    /// Regenerate the `omnia-sdk` guest bindings from the WIT.
    Bindgen,
    /// Fail if a copy, a package, or the bindings are out of step with the WIT.
    Check {
        /// A command component built against `omnia-sdk`, which must target
        /// `omnia:sdk/command`.
        #[arg(value_parser = |guest: &str| path::absolute(guest))]
        guest: PathBuf,
    },
    /// Fail if a release the registry holds differs from the tree.
    Verify,
    /// Verify, then publish every release the registry lacks.
    Publish,
}

#[expect(clippy::print_stderr, reason = "the failure a maintainer acts on")]
fn main() -> ExitCode {
    let Cli { command } = Cli::parse();
    match run(command) {
        Ok(()) => ExitCode::SUCCESS,
        Err(err) => {
            eprintln!("{err:#}");
            ExitCode::FAILURE
        }
    }
}

// the workspace root, which every path the commands name is relative to
const ROOT: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/..");

fn run(command: Command) -> Result<()> {
    env::set_current_dir(ROOT).context("cannot enter the workspace root")?;
    let workspace = Workspace::load()?;
    match command {
        Command::Sync => sync(&workspace),
        Command::Bindgen => Sdk::new(&workspace, &workspace.packages()?, true)?.bindings()?.write(),
        Command::Check { guest } => check(&workspace, &guest),
        Command::Verify => verify(&workspace, &Registry::open()?).map(drop),
        Command::Publish => publish(&workspace, &Registry::open()?),
    }
}

#[expect(clippy::print_stdout, reason = "each copy it rewrites")]
fn sync(workspace: &Workspace) -> Result<()> {
    for (copy, owner) in workspace.sync()? {
        println!("synced {} from {}", copy.display(), owner.display());
    }
    Ok(())
}

fn check(workspace: &Workspace, guest: &Path) -> Result<()> {
    // the WIT as written
    let mut problems: Vec<_> = workspace
        .stale_copies()?
        .into_iter()
        .map(|(copy, owner)| {
            format!("{} differs from {}: run `mise run wit-sync`", copy.display(), owner.display())
        })
        .collect();
    let comments = package::comments(&workspace.published)?;
    if !comments.is_empty() {
        problems.extend(comments);
        problems.push(
            "a plain `//` comment is published as docs: write `///` docs, or keep the note out of \
             the WIT"
                .to_owned(),
        );
    }
    for dir in &workspace.wit_dirs {
        problems.extend(package::unresolved(dir));
    }
    ensure!(problems.is_empty(), problems.join("\n"));

    // the packages as published
    let packages = workspace.packages()?;
    for package in &packages {
        package.encode(workspace)?;
    }

    // the bindings as committed, and a guest built from them
    Sdk::new(workspace, &packages, false)
        .context("`omnia:sdk` does not resolve with no features")?;
    let sdk = Sdk::new(workspace, &packages, true)?;
    let stale = sdk.bindings()?.stale();
    ensure!(stale.is_empty(), stale.join("\n"));
    sdk.targets(&fs::read(guest).with_context(|| format!("cannot read {}", guest.display()))?)
}

#[expect(clippy::print_stdout, reason = "each release's state")]
fn verify(workspace: &Workspace, registry: &Registry) -> Result<Vec<Release>> {
    // `omnia:types` first, since every other package uses it
    let mut packages = workspace.packages()?;
    packages.sort_by_key(|package| package.name.name != "types");

    let encoded = packages
        .iter()
        .map(|package| Ok((package.name.clone(), package.encode(workspace)?)))
        .collect::<Result<_>>()?;
    let releases = registry.releases(encoded)?;
    for release in &releases {
        println!("{release}");
    }
    ensure!(
        !releases.iter().any(|release| matches!(release.state, State::Differs { .. })),
        "a published release differs from the tree: give each package that differs a new version"
    );
    Ok(releases)
}

#[expect(clippy::print_stdout, reason = "each release it publishes")]
fn publish(workspace: &Workspace, registry: &Registry) -> Result<()> {
    for release in verify(workspace, registry).context("nothing published")? {
        if matches!(release.state, State::Absent) {
            registry.publish(&release)?;
            println!("published {}", release.package);
        }
    }
    Ok(())
}

// the first line, counting from 1, at which `a` and `b` part
fn parting_line(a: &str, b: &str) -> usize {
    a.lines().zip(b.lines()).take_while(|(a, b)| a == b).count() + 1
}
