//! A span tree reaches the host intact, in the host's trace:
//!
//! - The instrumented root takes the host's span as its parent, so the trace
//!   id the guest sees is the host's.
//! - A `tracing` child carries its field, its error event, and the error
//!   status the event implies.
//! - An instrumented sync function opens its span under the root without
//!   owning the telemetry lifecycle.
//! - A span opened through the OpenTelemetry API inside the root joins the
//!   same tree in its own instrumentation scope, with its schema URL and a
//!   link to a remote context with its `tracestate`.

#![cfg(target_arch = "wasm32")]

use omnia_sdk::otel::traces;
use opentelemetry::trace::{
    SpanContext, SpanId, TraceContextExt as _, TraceFlags, TraceId, TraceState, Tracer as _,
};
use opentelemetry::{Context, InstrumentationScope, KeyValue};
use tracing::{Instrument as _, Level};

omnia_sdk::api::command::export!(scenario);

async fn scenario() {
    parent().await;
}

// ERROR-level throughout: every span must pass the guest's default
// `EnvFilter` (the test environment sets no `RUST_LOG`).
#[omnia_sdk::otel::instrument(level = Level::ERROR)]
async fn parent() {
    let host = traces::current_span_context().expect("the host has a span for the run");
    let current = Context::current().span().span_context().trace_id();
    assert_eq!(current.to_string(), host.trace_id, "the root joins the host's trace");

    async {
        tracing::error!("child failed");
    }
    .instrument(tracing::error_span!("child", answer = 42))
    .await;

    sibling();

    // the entered parent is the current OpenTelemetry context, so the API
    // span parents to it without any explicit wiring
    let scope = InstrumentationScope::builder("guest").with_schema_url("https://guest").build();
    opentelemetry::global::tracer_with_scope(scope).in_span("api", |cx| {
        cx.span().set_attribute(KeyValue::new("kind", "direct"));
        cx.span().add_link(
            SpanContext::new(
                TraceId::from_hex("0102030405060708090a0b0c0d0e0f10").expect("trace id"),
                SpanId::from_hex("0102030405060708").expect("span id"),
                TraceFlags::SAMPLED,
                true,
                TraceState::from_key_value([("vendor", "opaque"), ("other", "a:b")])
                    .expect("tracestate"),
            ),
            Vec::new(),
        );
    });
}

#[omnia_sdk::otel::instrument(level = Level::ERROR)]
fn sibling() {}
