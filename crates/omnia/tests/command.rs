//! Exit-status integration test for command mode ([`omnia::run`] in command mode).
//!
//! Builds a minimal runtime over the `cli-wasm` example guest and drives it
//! exactly as a one-shot command deployment would, asserting the exit status
//! for each subcommand — including the nonzero paths: a specific code carried by
//! `wasi:cli/exit` (surfaced as `I32Exit`, proving codes are *not* collapsed to
//! `1`) and the `Err(())` -> `1` mapping.
//!
//! The guest is built by `cargo make build-guests`; the test skips locally when
//! it is absent and fails under CI so the pipeline never passes vacuously.

#![cfg(not(target_arch = "wasm32"))]

mod common;

use std::path::Path;

use anyhow::{Context as _, Result};
use common::find_guest;
use omnia::{Deployment, ExitStatus, Mode, Runtime, StoreCtx, Wiring, run};

struct EmptyWiring;

impl Wiring<()> for EmptyWiring {
    fn link(_deployment: &mut Deployment<StoreCtx<()>>) -> Result<()> {
        Ok(())
    }

    async fn serve(_runtime: &Runtime<()>) -> Result<()> {
        Ok(())
    }
}

/// Drive `wasi:cli/run` once with `tail` guest argv (the program name is
/// prepended by command mode) and return the guest's exit status.
async fn run_cli(wasm: &Path, tail: &[&str]) -> Result<ExitStatus> {
    // The `()` bundle links no hosts; `wasi:cli` is wired by the deployment
    // builder, and `Runtime::new` threads the guest argv into every store.
    run::<(), EmptyWiring>(
        Some(wasm.to_path_buf()),
        None,
        tail.iter().map(|arg| (*arg).to_string()).collect(),
        Mode::Command,
    )
    .await
    .context("running command")
}

macro_rules! cli_exit_test {
    ($name:ident, $tail:expr, $code:expr, $msg:expr) => {
        #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
        async fn $name() -> Result<()> {
            let Some(wasm) = find_guest("cli_wasm.wasm", "cargo make build-guests") else {
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
