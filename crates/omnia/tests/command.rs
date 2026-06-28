//! Exit-status integration test for command mode (`omnia::run_command`).
//!
//! Builds a minimal runtime over the `cli-wasm` example guest and drives it
//! exactly as a one-shot command deployment would, asserting the exit status
//! for each subcommand — including the nonzero paths: a specific code carried by
//! `wasi:cli/exit` (surfaced as `I32Exit`, proving codes are *not* collapsed to
//! `1`) and the `Err(())` -> `1` mapping.
//!
//! The guest must be built first; the test skips (rather than fails) when it is
//! absent, because `cargo make ci` cleans the target directory before tests:
//!
//! ```bash
//! cargo build -p examples --example cli-wasm --target wasm32-wasip2
//! ```

#![cfg(not(target_arch = "wasm32"))]

use std::path::{Path, PathBuf};
use std::sync::Arc;

use anyhow::{Context as _, Result};
use omnia::{ExitStatus, Registry, RegistryBuilder, Runtime, StoreBase};

/// Per-store context: just the fixed [`StoreBase`]. The `wasi:cli` guest needs
/// only the WASI view, which the `StoreContext` derive supplies from `base`.
#[derive(omnia::StoreContext)]
struct TestCtx {
    #[base]
    base: StoreBase,
}

/// A minimal [`Runtime`] over a single `wasi:cli` guest, threading a fixed argv
/// into every store (the guest reads it as `wasi:cli/environment`). `args[0]` is
/// the program name, as command mode prepends.
#[derive(Clone)]
struct TestRuntime {
    registry: Arc<Registry<TestCtx>>,
    args: Arc<Vec<String>>,
}

impl Runtime for TestRuntime {
    type StoreCtx = TestCtx;

    fn store(&self) -> TestCtx {
        TestCtx {
            base: StoreBase::builder()
                .options(self.options())
                .dispatch(Arc::new(self.clone()))
                .args(&self.args)
                .build(),
        }
    }

    fn registry(&self) -> &Registry<Self::StoreCtx> {
        &self.registry
    }
}

/// The `target/` directory: the test executable lives at
/// `<target>/<profile>/deps/<exe>`.
fn target_dir() -> PathBuf {
    let exe = std::env::current_exe().expect("test executable has a path");
    exe.ancestors().nth(3).expect("test exe sits at <target>/<profile>/deps/<exe>").to_path_buf()
}

/// Locate the built `cli-wasm` guest, preferring the debug profile.
fn cli_wasm(target: &Path) -> Option<PathBuf> {
    ["debug", "release"]
        .into_iter()
        .map(|profile| {
            target.join("wasm32-wasip2").join(profile).join("examples").join("cli_wasm.wasm")
        })
        .find(|path| path.exists())
}

/// Compile and assemble a single-guest registry for `wasm`. Done once per test:
/// `compile` initializes the process-global telemetry, which can only be set
/// once, so each subcommand reuses this registry (instance-per-call regardless).
async fn build_registry(wasm: &Path) -> Result<Arc<Registry<TestCtx>>> {
    let compiled = RegistryBuilder::new()
        .wasm(wasm.to_path_buf())
        .command(true)
        .compile::<TestCtx>()
        .await
        .context("building runtime")?;
    Ok(Arc::new(compiled.build().context("assembling registry")?))
}

/// Drive `wasi:cli/run` once over `registry` with `tail` (the program name is
/// prepended as `args[0]`) and return the guest's exit status.
async fn run_cli(registry: &Arc<Registry<TestCtx>>, tail: &[&str]) -> Result<ExitStatus> {
    let mut argv = vec![String::from("cli_wasm")];
    argv.extend(tail.iter().map(|arg| (*arg).to_owned()));
    let runtime = TestRuntime {
        registry: Arc::clone(registry),
        args: Arc::new(argv),
    };

    omnia::run(&runtime, true, vec![]).await.context("running command")
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn cli_exit_status_matches_subcommand() -> Result<()> {
    let target = target_dir();
    let Some(wasm) = cli_wasm(&target) else {
        eprintln!(
            "skipping `cli_exit_status_matches_subcommand`: cli guest not built. Run:\n  \
             cargo build -p examples --example cli-wasm --target wasm32-wasip2"
        );
        return Ok(());
    };

    let registry = build_registry(&wasm).await?;

    // A known subcommand returns `Ok(())` -> success (0).
    assert_eq!(run_cli(&registry, &["greet", "Ada"]).await?.code(), 0, "greet exits 0");
    assert_eq!(run_cli(&registry, &["add", "2", "40"]).await?.code(), 0, "add exits 0");

    // A specific nonzero code rides `wasi:cli/exit` -> `I32Exit` -> the status,
    // proving codes survive rather than collapsing to 1.
    assert_eq!(run_cli(&registry, &["bogus"]).await?.code(), 2, "unknown command exits 2");

    // A plain `Err(())` from `run` (missing subcommand) maps to 1.
    assert_eq!(run_cli(&registry, &[]).await?.code(), 1, "missing subcommand exits 1");

    Ok(())
}
