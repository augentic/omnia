//! The guest's `wasi:cli` environment through the real runtime: `RUST_LOG`
//! is the deployment's tracing level — the level selected for the run, else
//! the process's own, else the command-mode default.

#![cfg(not(target_arch = "wasm32"))]

use omnia::{ExitStatus, LevelFilter};
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

// The test process's `RUST_LOG` is read, never set, so the suite never
// writes its own environment: a set variable stands, an unset one falls
// back to the command-mode `info`.
#[tokio::test]
async fn cli_rust_log() {
    let expected = std::env::var("RUST_LOG").unwrap_or_else(|_| "info".to_owned());
    assert_eq!(run(Deployment::new(), expected).await, ExitStatus::SUCCESS);
}

#[tokio::test]
async fn cli_rust_log_level() {
    let deployment = Deployment::new().level(LevelFilter::DEBUG);
    assert_eq!(run(deployment, "debug".to_owned()).await, ExitStatus::SUCCESS);
}
