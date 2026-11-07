//! An instrumented function's span reaches the host: `command::export!` owns the
//! telemetry lifecycle here, so `traced` records into it and the flush at
//! the end of the run exports the span.

#![cfg(target_arch = "wasm32")]

use tracing::Level;

omnia_sdk::api::command::export!(scenario);

async fn scenario() {
    traced().await;
}

// ERROR-level: the span must pass the guest's default `EnvFilter` (the test
// environment sets no `RUST_LOG`).
#[omnia_wasi_otel::instrument(level = Level::ERROR)]
async fn traced() {
    sync_traced();
}

#[omnia_wasi_otel::instrument(level = Level::ERROR)]
fn sync_traced() {}
