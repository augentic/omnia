//! Every `omnia:otel` error case reaches the guest as its own case.
//!
//! - The host refuses telemetry that is not portable as `invalid`, naming
//!   the span and the field, before any of it reaches the backend.
//! - The suite's faulting backend fails the rest by the name of the first
//!   span or metric.

#![cfg(target_arch = "wasm32")]

use omnia_sdk::otel::metrics::{
    self, Exemplar, Gauge, Metric, MetricData, Number, NumberDataPoint, ScopeMetrics,
};
use omnia_sdk::otel::traces::{self, ScopeSpans, Span, SpanContext, SpanKind, Status};
use omnia_sdk::otel::types::{AnyValue, Error, KeyValue};

omnia_sdk::command::export!(scenario);

async fn scenario() {
    // the host's refusals
    let mut not_hex = span("not-hex");
    not_hex.span_context.trace_id = "0102030405060708090a0b0c0d0e0fzz".to_owned();
    let refused = export_span(not_hex).await;
    let Err(Error::Invalid(message)) = &refused else {
        panic!("a trace id that is not hexadecimal is `invalid`, got {refused:?}");
    };
    assert!(message.contains("span `not-hex`") && message.contains("trace id"), "{message}");

    let mut uppercase = span("uppercase");
    uppercase.span_context.span_id = "111213141516171A".to_owned();
    assert_eq!(case(export_span(uppercase).await), "invalid");

    let mut zeros = span("zeros");
    zeros.span_context.trace_id = "0".repeat(32);
    assert_eq!(case(export_span(zeros).await), "invalid");

    let mut short_parent = span("short-parent");
    short_parent.parent_span_id = Some("2122".to_owned());
    assert_eq!(case(export_span(short_parent).await), "invalid");

    let mut not_json = span("not-json");
    not_json.attributes = vec![KeyValue {
        key: "json".to_owned(),
        value: AnyValue::JsonValue("{not json".to_owned()),
    }];
    assert_eq!(case(export_span(not_json).await), "invalid");

    let mut exemplar = metric("exemplar");
    let MetricData::Gauge(gauge) = &mut exemplar.data else {
        unreachable!("`metric` builds a gauge");
    };
    gauge.data_points[0].exemplars = vec![Exemplar {
        filtered_attributes: Vec::new(),
        time_unix_nano: 1,
        value: Number::AsInt(1),
        span_id: Some("not-hex".to_owned()),
        trace_id: None,
    }];
    assert_eq!(case(export_metric(exemplar).await), "invalid");

    // the backend's chosen cases
    for name in ["not-found", "denied", "invalid", "unsupported", "unavailable", "other"] {
        assert_eq!(case(export_span(span(name)).await), name);
    }
    assert_eq!(case(export_metric(metric("unavailable")).await), "unavailable");
}

fn span(name: &str) -> Span {
    Span {
        span_context: SpanContext {
            trace_id: "0102030405060708090a0b0c0d0e0f10".to_owned(),
            span_id: "1112131415161718".to_owned(),
            trace_flags: 0x01,
            trace_state: None,
        },
        parent_span_id: None,
        parent_is_remote: None,
        name: name.to_owned(),
        kind: SpanKind::Internal,
        start_time_unix_nano: 1,
        end_time_unix_nano: 2,
        attributes: Vec::new(),
        dropped_attributes_count: 0,
        events: Vec::new(),
        dropped_events_count: 0,
        links: Vec::new(),
        dropped_links_count: 0,
        status: Status::Unset,
    }
}

async fn export_span(span: Span) -> Result<(), Error> {
    traces::export(vec![ScopeSpans {
        scope: None,
        spans: vec![span],
        schema_url: None,
    }])
    .await
}

fn metric(name: &str) -> Metric {
    Metric {
        name: name.to_owned(),
        description: None,
        unit: None,
        data: MetricData::Gauge(Gauge {
            data_points: vec![NumberDataPoint {
                attributes: Vec::new(),
                start_time_unix_nano: None,
                time_unix_nano: 1,
                value: Number::AsInt(1),
                exemplars: Vec::new(),
            }],
        }),
    }
}

async fn export_metric(metric: Metric) -> Result<(), Error> {
    metrics::export(vec![ScopeMetrics {
        scope: None,
        metrics: vec![metric],
        schema_url: None,
    }])
    .await
}

fn case(result: Result<(), Error>) -> &'static str {
    match result.expect_err("the call fails") {
        Error::NotFound(_) => "not-found",
        Error::Denied(_) => "denied",
        Error::Invalid(_) => "invalid",
        Error::Unsupported(_) => "unsupported",
        Error::Unavailable(_) => "unavailable",
        Error::Other(_) => "other",
    }
}
