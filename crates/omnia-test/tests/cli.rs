//! The guest's `wasi:cli` environment through the real runtime: the
//! deployment's `[env]` defaults fill what the host process lacks and yield
//! to what it sets.

#![cfg(not(target_arch = "wasm32"))]

use omnia::ExitStatus;
use omnia_test::host::{Backends, Deployment};
use omnia_wasi_otel::WasiOtel;

test_programs::foreach_cli!();

// The shadowed variable is one the test process already has, read rather
// than set, so the suite never writes its own environment.
#[tokio::test]
async fn cli_env() {
    let (shadowed, host_value) = std::env::vars()
        .find(|(name, _)| name != "OMNIA_TEST_DEFAULT")
        .expect("the test process has an environment");

    let status = Deployment::new()
        .guest("cli", test_programs::CLI_ENV)
        .env([("OMNIA_TEST_DEFAULT", "from-manifest"), (shadowed.as_str(), "from-manifest")])
        .args([shadowed, host_value])
        .run(Backends::defaults().await, |deployment| {
            deployment.host::<WasiOtel, Backends>()?;
            Ok(())
        })
        .await
        .expect("deployment runs");

    assert_eq!(status, ExitStatus::SUCCESS);
}
