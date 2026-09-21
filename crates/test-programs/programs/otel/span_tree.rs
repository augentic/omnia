//! A span tree reaches the host intact: a `tracing` child under an
//! instrumented parent carries its field, its error event, and the error
//! status the event implies, and a span opened through the OpenTelemetry API
//! inside the parent joins the same tree in its own instrumentation scope.

#![cfg(target_arch = "wasm32")]

use opentelemetry::KeyValue;
use opentelemetry::trace::{TraceContextExt as _, Tracer as _};
use tracing::{Instrument as _, Level};

omnia_sdk::command!(scenario);

async fn scenario() {
    parent().await;
}

// ERROR-level throughout: every span must pass the guest's default
// `EnvFilter` (the test environment sets no `RUST_LOG`).
#[omnia_wasi_otel::instrument(level = Level::ERROR)]
async fn parent() {
    async {
        tracing::error!("child failed");
    }
    .instrument(tracing::error_span!("child", answer = 42))
    .await;

    // The entered parent is the current OpenTelemetry context, so the API
    // span parents to it without any explicit wiring.
    opentelemetry::global::tracer("guest").in_span("api", |cx| {
        cx.span().set_attribute(KeyValue::new("kind", "direct"));
    });
}
