//! The guest's `wasi:cli` environment through the real runtime: `RUST_LOG`
//! is the deployment's tracing directives — the level selected for the run
//! (else the process's own, else the command-mode default) composed with the
//! process `RUST_LOG`'s targeted directives.

#![cfg(not(target_arch = "wasm32"))]

use omnia::{ExitStatus, LevelFilter, telemetry};
use omnia_test::host::{Backends, Deployment};
use omnia_wasi_otel::WasiOtel;

test_programs::foreach_cli!();

async fn run(deployment: Deployment, expected: String) -> ExitStatus {
    deployment
        .guest("cli", test_programs::CLI_RUST_LOG)
        .args([expected])
        .run(Backends::defaults().await, |deployment| {
            deployment.host::<WasiOtel, Backends>()?;
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
    assert_eq!(run(Deployment::new(), expected(None)).await, ExitStatus::SUCCESS);
}

#[tokio::test]
async fn cli_rust_log_level() {
    let deployment = Deployment::new().level(LevelFilter::DEBUG);
    let expected = expected(Some(LevelFilter::DEBUG));
    assert!(expected.starts_with("debug"), "the selected level leads: {expected}");
    assert_eq!(run(deployment, expected).await, ExitStatus::SUCCESS);
}
