//! A span tree reaches the host intact: a `tracing` child under an
//! instrumented parent carries its field, its error event, and the error
//! status the event implies, an instrumented sync function opens its span
//! under the parent without owning the telemetry lifecycle, a span opened
//! through the OpenTelemetry API inside the parent joins the same tree in its
//! own instrumentation scope with its schema URL and linked `tracestate`, and
//! a raw `omnia:otel` export with malformed ids and overflowing timestamps
//! clamps cleanly on the host.

#![cfg(target_arch = "wasm32")]

mod raw {
    wit_bindgen::generate!({
        inline: "
            package omnia-test:otel;
            world raw-otel {
                import omnia:otel/tracing@0.1.0;
            }
        ",
        path: "../wasi-otel/wit",
        generate_all,
    });
}

use opentelemetry::trace::{
    SpanContext, SpanId, TraceContextExt as _, TraceFlags, TraceId, TraceState, Tracer as _,
};
use opentelemetry::{InstrumentationScope, KeyValue};
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

    export_malformed().await;
}

#[omnia_wasi_otel::instrument(level = Level::ERROR)]
fn sibling() {}

async fn export_malformed() {
    use raw::omnia::otel::tracing::{
        Link, SpanContext, SpanData, SpanKind, Status, TraceFlags, export,
    };
    use raw::omnia::otel::types::InstrumentationScope;
    use raw::wasi::clocks::wall_clock::Datetime;

    let max_time = Datetime {
        seconds: u64::MAX,
        nanoseconds: 999_999_999,
    };
    export(vec![SpanData {
        span_context: SpanContext {
            trace_id: String::new(),
            span_id: "not-hex".to_owned(),
            trace_flags: TraceFlags::SAMPLED,
            is_remote: false,
            trace_state: vec![
                ("vendor".to_owned(), "opaque".to_owned()),
                ("other".to_owned(), "a:b=c".to_owned()),
            ],
        },
        parent_span_id: "abc".to_owned(),
        span_kind: SpanKind::Internal,
        name: "malformed".to_owned(),
        start_time: max_time,
        end_time: max_time,
        attributes: Vec::new(),
        events: Vec::new(),
        links: vec![Link {
            span_context: SpanContext {
                trace_id: "not-hex".to_owned(),
                span_id: "abc".to_owned(),
                trace_flags: TraceFlags::empty(),
                is_remote: true,
                trace_state: Vec::new(),
            },
            attributes: Vec::new(),
        }],
        status: Status::Unset,
        instrumentation_scope: InstrumentationScope {
            name: "raw".to_owned(),
            version: None,
            schema_url: None,
            attributes: Vec::new(),
        },
        dropped_attributes: 0,
        dropped_events: 0,
        dropped_links: 0,
    }])
    .await
    .expect("raw export");
}
