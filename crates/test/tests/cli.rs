//! The guest's `wasi:cli` environment through the real runtime:
//!
//! - `RUST_LOG` is the deployment's tracing directives — the level selected
//!   for the run (else the process's own, else the command-mode default)
//!   composed with the process `RUST_LOG`'s targeted directives;
//! - every other variable is one the deployment names, read from the host.

#![cfg(not(target_arch = "wasm32"))]

use omnia::{ExitStatus, LevelFilter, telemetry};
use omnia_test::host::{Backends, Deployment, precompiled};
use omnia_wasi_otel::WasiOtel;

test_programs::foreach_cli!();

// each program here boots more than once, so it is compiled once
async fn run(deployment: Deployment, program: &str, args: Vec<String>) -> ExitStatus {
    deployment
        .guest("cli", precompiled(program).expect("the guest precompiles"))
        .args(args)
        .run(Backends::defaults().await, |deployment| {
            deployment.host::<WasiOtel>()?;
            Ok(())
        })
        .await
        .expect("deployment runs")
}

// The expectation the runtime composes for a command run: the test process's
// `RUST_LOG` is read, never set, so the suite never writes its own
// environment. A set variable stands (its bare level displaced by a selected
// one), an unset one falls back to the command-mode `info`.
fn expected(level: Option<LevelFilter>) -> String {
    telemetry::directives(level, LevelFilter::INFO, std::env::var("RUST_LOG").ok().as_deref())
}

#[tokio::test]
async fn cli_rust_log() {
    let status = run(Deployment::new(), test_programs::CLI_RUST_LOG, vec![expected(None)]).await;
    assert_eq!(status, ExitStatus::SUCCESS);
}

#[tokio::test]
async fn selected_level() {
    let deployment = Deployment::new().level(LevelFilter::DEBUG);
    let expected = expected(Some(LevelFilter::DEBUG));
    assert!(expected.starts_with("debug"), "the selected level leads: {expected}");
    let status = run(deployment, test_programs::CLI_RUST_LOG, vec![expected]).await;
    assert_eq!(status, ExitStatus::SUCCESS);
}

// The test process's variables `select` keeps, as `NAME=value` pairs: the
// ones the runtime can pass, since a pair that is not UTF-8 cannot cross
// `wasi:cli`, less the two it sets itself.
fn host_pairs(select: impl Fn(&str) -> bool) -> Vec<String> {
    std::env::vars_os()
        .filter_map(|(name, value)| Some((name.into_string().ok()?, value.into_string().ok()?)))
        .filter(|(name, _)| name != "RUST_LOG" && name != "NO_COLOR" && select(name))
        .map(|(name, value)| format!("{name}={value}"))
        .collect()
}

// Cargo and nextest set `CARGO_MANIFEST_DIR` and the `CARGO_PKG_*` family on
// every test process, and `PATH` and the rest stay with the host.
#[tokio::test]
async fn cli_env() {
    let expected =
        host_pairs(|name| name == "CARGO_MANIFEST_DIR" || name.starts_with("CARGO_PKG_"));
    assert!(
        expected.iter().any(|pair| pair.starts_with("CARGO_PKG_NAME=")),
        "the test process carries `CARGO_PKG_NAME`: {expected:?}"
    );

    let deployment = Deployment::new().inherit(["CARGO_MANIFEST_DIR", "CARGO_PKG_*"]);
    let status = run(deployment, test_programs::CLI_ENV, expected).await;
    assert_eq!(status, ExitStatus::SUCCESS);

    let status = run(Deployment::new(), test_programs::CLI_ENV, Vec::new()).await;
    assert_eq!(status, ExitStatus::SUCCESS, "a deployment that names none passes none");
}

#[tokio::test]
async fn inherit_all() {
    let deployment = Deployment::new().inherit(["*"]);
    let status = run(deployment, test_programs::CLI_ENV, host_pairs(|_| true)).await;
    assert_eq!(status, ExitStatus::SUCCESS);
}
