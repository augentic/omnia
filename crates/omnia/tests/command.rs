//! Exit-status integration test for command mode ([`omnia::run`] in command mode).
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
use omnia::{ExitStatus, Registry, Runtime, StoreBase, run};

/// Per-store context: the library [`omnia::StoreCtx`] over the empty `()` backend
/// bundle. The `wasi:cli` guest needs only the WASI view, which `StoreCtx`
/// supplies from its `base`.
type TestCtx = omnia::StoreCtx<()>;

/// A minimal [`Runtime`] over a single `wasi:cli` guest, threading guest argv
/// into every store (the guest reads it as `wasi:cli/environment`). In command
/// mode the floor prepends the deployment name as `args[0]`.
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
            backends: (),
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

/// Drive `wasi:cli/run` once with `tail` guest argv (the program name is
/// prepended by command mode) and return the guest's exit status.
async fn run_cli(wasm: &Path, tail: &[&str]) -> Result<ExitStatus> {
    run(
        Some(wasm.to_path_buf()),
        None,
        tail.iter().map(|arg| (*arg).to_string()).collect(),
        true,
        |compiled| async move {
            let args = Arc::new(compiled.args().to_vec());
            Ok(TestRuntime {
                registry: Arc::new(compiled.build().context("assembling registry")?),
                args,
            })
        },
        |_| vec![],
    )
    .await
    .context("running command")
}

macro_rules! cli_exit_test {
    ($name:ident, $tail:expr, $code:expr, $msg:expr) => {
        #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
        async fn $name() -> Result<()> {
            let target = target_dir();
            let Some(wasm) = cli_wasm(&target) else {
                eprintln!(
                    "skipping `{}`: cli guest not built. Run:\n  cargo build -p examples \
                     --example cli-wasm --target wasm32-wasip2",
                    stringify!($name)
                );
                return Ok(());
            };

            assert_eq!(run_cli(&wasm, $tail).await?.code(), $code, $msg);
            Ok(())
        }
    };
}

cli_exit_test!(greet_exits_zero, &["greet", "Ada"], 0, "greet exits 0");
cli_exit_test!(add_exits_zero, &["add", "2", "40"], 0, "add exits 0");
cli_exit_test!(unknown_command_exits_two, &["bogus"], 2, "unknown command exits 2");
cli_exit_test!(missing_subcommand_exits_one, &[], 1, "missing subcommand exits 1");
