//! The telemetry flush completes while a spawned task is still pending.
//!
//! Regression: the flush used to run inside `wit_bindgen::block_on`, whose
//! fresh event loop stole the globally-queued spawned task and then waited on
//! it — but the task (here the oneshot receiver, in production an HTTP
//! response-body writer) can only finish after the instrumented function
//! returns, so the guest deadlocked.

#![cfg(target_arch = "wasm32")]

use futures::channel::oneshot;
use tracing::Level;

struct CliGuest;

wasip3::cli::command::export!(CliGuest);

// Not `omnia_sdk::command!`: that opens the telemetry scope around the whole
// scenario, so `traced` would neither own the lifecycle nor flush while the
// receiver is pending — the exact condition this regression needs.
impl wasip3::exports::cli::run::Guest for CliGuest {
    async fn run() -> Result<(), ()> {
        scenario().await;
        Ok(())
    }
}

async fn scenario() {
    let (tx, rx) = oneshot::channel::<()>();
    wit_bindgen::spawn_local(async move {
        let _ = rx.await;
    });
    // The outermost instrumented call owns the lifecycle, so telemetry
    // flushes as it returns — while the spawned receiver is still pending.
    traced().await;
    // Never reached when the flush deadlocks.
    let _ = tx.send(());
}

// ERROR-level: the span must pass the guest's default `EnvFilter` (the test
// environment sets no `RUST_LOG`).
#[omnia_wasi_otel::instrument(level = Level::ERROR)]
async fn traced() {}
