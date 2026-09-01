//! An instrumented function owning the telemetry lifecycle exports its span
//! to the host as it returns.

#![cfg(target_arch = "wasm32")]

use tracing::Level;

test_programs::run!(scenario);

async fn scenario() {
    traced().await;
}

// ERROR-level: the span must pass the guest's default `EnvFilter` (the test
// environment sets no `RUST_LOG`).
#[omnia_wasi_otel::instrument(level = Level::ERROR)]
async fn traced() {}
