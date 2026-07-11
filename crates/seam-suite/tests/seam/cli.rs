//! Composed WASI parity for the guest command router, driven exactly as a
//! one-shot command deployment would.
//!
//! One table-driven test covers every operation route plus router-generated
//! help, version, and usage behavior, including arbitrary nonzero codes
//! carried by p3 `wasi:cli/exit`. Each case runs the full public `run()` path
//! (deployment build, command routing, exit mapping); the serialized guest
//! artifact keeps the per-case build cheap.

use std::path::Path;

use anyhow::{Context as _, Result};
use omnia::{Deployment, DeploymentBuilder, ExitStatus, Mode, Runtime, StoreCtx, Wiring, run};
use omnia_testkit::find_guest;

use crate::fixture;

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
    let builder = DeploymentBuilder::new()
        .wasm(wasm.to_path_buf())
        .args(tail.iter().map(|arg| (*arg).to_string()).collect::<Vec<_>>())
        .mode(Mode::Command);
    run::<(), EmptyWiring>(builder).await.context("running command")
}

#[test]
fn exit_codes() -> Result<()> {
    fixture::RT.block_on(async {
        let wasm = find_guest("cli_wasm.wasm");

        let cases: &[(&[&str], i32, &str)] = &[
            (&["greet", "Ada"], 0, "greet exits 0"),
            (&["greet"], 0, "default greeting exits 0"),
            (&["add", "2", "40"], 0, "add exits 0"),
            (&["env"], 0, "env exits 0"),
            (&["--help"], 0, "clap-generated --help exits 0"),
            (&["--version"], 0, "clap-generated --version exits 0"),
            (&["bogus"], 2, "clap usage error exits 2"),
            (&[], 2, "clap usage error exits 2"),
            (&["fail", "42"], 42, "wasi:cli/exit carries a specific code"),
            (&["fail"], 1, "Err(()) from run maps to 1"),
        ];

        for (tail, code, expectation) in cases {
            let status = run_cli(&wasm, tail).await?;
            assert_eq!(status.code(), *code, "{expectation} (argv: {tail:?})");
        }

        Ok(())
    })
}
