//! An instrumented `async fn` is usable as an axum handler. axum's `Handler`
//! bound requires a `Send` future, and the one `#[instrument]` returns spans
//! `scope` and its export awaits; the guest build checks the bound, and the
//! run exports the span through that same function.

#![cfg(target_arch = "wasm32")]

use axum::handler::Handler;
use tracing::Level;

omnia_sdk::command!(scenario);

async fn scenario() {
    fn assert_handler<H: Handler<T, ()>, T>(handler: H) -> H {
        handler
    }
    assert_handler(traced)().await;
}

// ERROR-level: the span must pass the guest's default `EnvFilter` (the test
// environment sets no `RUST_LOG`).
#[omnia_wasi_otel::instrument(level = Level::ERROR)]
async fn traced() -> &'static str {
    "traced"
}
