//! End-to-end tests for `wasi:config/store`: every scenario runs a real guest
//! component from `crates/test-programs` through the omnia runtime against
//! the map-backed default seeded by `Backends::config`.

#![cfg(not(target_arch = "wasm32"))]

use omnia::ExitStatus;
use omnia_test::host::{Backends, Deployment};
use omnia_wasi_config::WasiConfig;

// Every guest program in `crates/test-programs` must have a matching test
// here; a new program without one fails to compile.
test_programs::foreach_config!();

/// Run one guest program against `backends`, requiring a clean exit.
async fn run_guest(wasm: &str, backends: Backends) {
    let status = Deployment::new()
        .guest("guest", wasm)
        .run_host::<WasiConfig, _>(backends)
        .await
        .expect("guest runs");
    assert_eq!(status, ExitStatus::SUCCESS, "guest `{wasm}` failed");
}

#[tokio::test]
async fn config_seeded() {
    let backends = Backends::defaults().await.config([("GREETING", "hello")]);
    run_guest(test_programs::CONFIG_SEEDED, backends).await;
}
