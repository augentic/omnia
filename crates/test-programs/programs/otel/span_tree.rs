//! A span tree reaches the host intact: a `tracing` child under an
//! instrumented parent carries its field, its error event, and the error
//! status the event implies, and a span opened through the OpenTelemetry API
//! inside the parent joins the same tree in its own instrumentation scope.

#![cfg(target_arch = "wasm32")]

use opentelemetry::KeyValue;
use opentelemetry::trace::{TraceContextExt as _, Tracer as _};
use tracing::{Instrument as _, Level};

omnia_sdk::api::command::export!(scenario);

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
    let link_context = opentelemetry::trace::SpanContext::new(
        opentelemetry::trace::TraceId::from_bytes([1; 16]),
        opentelemetry::trace::SpanId::from_bytes([2; 8]),
        opentelemetry::trace::TraceFlags::SAMPLED,
        true,
        "vendor=opaque,other=value".parse().expect("valid tracestate"),
    );
    opentelemetry::global::tracer("guest").in_span("api", |cx| {
        cx.span().set_attribute(KeyValue::new("kind", "direct"));
        cx.span().add_link(link_context, vec![]);
    });
}
