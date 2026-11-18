//! Telemetry the guest builds itself reaches the host as given, through the
//! raw `omnia:otel` bindings:
//!
//! - An empty export of spans or of metric points succeeds.
//! - A span carries its scope, its ids and flags, a parent from another
//!   process, its kind, every `any-value` case, its dropped counts, a link
//!   whose remoteness is unknown, and an error with no description.
//! - Metric points carry every kind of metric: a gauge with an exemplar and
//!   no start time, a cumulative sum, a histogram with no sum, and an
//!   exponential histogram with a negative scale.

#![cfg(target_arch = "wasm32")]

use omnia_wasi_otel::omnia::metrics::{
    self, AggregationTemporality, Buckets, Exemplar, ExponentialHistogram,
    ExponentialHistogramDataPoint, Gauge, Histogram, HistogramDataPoint, Metric, MetricData,
    Number, NumberDataPoint, ScopeMetrics, Sum,
};
use omnia_wasi_otel::omnia::traces::{
    self, Event, Link, ScopeSpans, Span, SpanContext, SpanKind, Status,
};
use omnia_wasi_otel::omnia::types::{AnyValue, InstrumentationScope, KeyValue};

omnia_sdk::api::command::export!(scenario);

const TRACE_ID: &str = "0102030405060708090a0b0c0d0e0f10";
const SPAN_ID: &str = "1112131415161718";

async fn scenario() {
    traces::export(Vec::new()).await.expect("an empty span export");
    metrics::export(Vec::new()).await.expect("an empty metrics export");

    traces::export(vec![ScopeSpans {
        scope: Some(InstrumentationScope {
            name: "raw".to_owned(),
            version: Some("1.2.3".to_owned()),
            attributes: vec![attribute("scope.attr", AnyValue::StringValue("x".to_owned()))],
            dropped_attributes_count: 2,
        }),
        spans: vec![span()],
        schema_url: Some("https://raw".to_owned()),
    }])
    .await
    .expect("span export");

    metrics::export(vec![ScopeMetrics {
        scope: None,
        metrics: vec![gauge(), sum(), histogram(), exponential_histogram()],
        schema_url: None,
    }])
    .await
    .expect("metrics export");
}

fn attribute(key: &str, value: AnyValue) -> KeyValue {
    KeyValue {
        key: key.to_owned(),
        value,
    }
}

fn span() -> Span {
    Span {
        span_context: SpanContext {
            trace_id: TRACE_ID.to_owned(),
            span_id: SPAN_ID.to_owned(),
            trace_flags: 0x03,
            trace_state: Some("vendor=opaque".to_owned()),
        },
        parent_span_id: Some("2122232425262728".to_owned()),
        parent_is_remote: Some(true),
        name: "raw".to_owned(),
        kind: SpanKind::Server,
        start_time_unix_nano: 1_000,
        end_time_unix_nano: 2_000,
        attributes: vec![
            attribute("string", AnyValue::StringValue("text".to_owned())),
            attribute("bool", AnyValue::BoolValue(true)),
            attribute("int", AnyValue::IntValue(-7)),
            attribute("double", AnyValue::DoubleValue(2.5)),
            attribute("bytes", AnyValue::BytesValue(vec![0, 255])),
            attribute("strings", AnyValue::StringArray(vec!["a".to_owned(), "b".to_owned()])),
            attribute("bools", AnyValue::BoolArray(vec![true, false])),
            attribute("ints", AnyValue::IntArray(vec![1, 2])),
            attribute("doubles", AnyValue::DoubleArray(vec![0.5])),
            attribute("json", AnyValue::JsonValue(r#"{"a":[1,"b",null,{"c":2.5}]}"#.to_owned())),
            attribute("empty", AnyValue::Empty),
        ],
        dropped_attributes_count: 3,
        events: vec![Event {
            time_unix_nano: 1_500,
            name: "tick".to_owned(),
            attributes: Vec::new(),
            dropped_attributes_count: 1,
        }],
        dropped_events_count: 4,
        links: vec![Link {
            span_context: SpanContext {
                trace_id: "3132333435363738393a3b3c3d3e3f40".to_owned(),
                span_id: "4142434445464748".to_owned(),
                trace_flags: 0x01,
                trace_state: None,
            },
            is_remote: None,
            attributes: Vec::new(),
            dropped_attributes_count: 0,
        }],
        dropped_links_count: 5,
        status: Status::Error(None),
    }
}

fn gauge() -> Metric {
    Metric {
        name: "temperature".to_owned(),
        description: Some("raw metrics".to_owned()),
        unit: None,
        data: MetricData::Gauge(Gauge {
            data_points: vec![NumberDataPoint {
                attributes: Vec::new(),
                start_time_unix_nano: None,
                time_unix_nano: 3_000,
                value: Number::AsDouble(21.5),
                exemplars: vec![Exemplar {
                    filtered_attributes: vec![attribute(
                        "probe",
                        AnyValue::StringValue("p1".to_owned()),
                    )],
                    time_unix_nano: 2_900,
                    value: Number::AsDouble(21.4),
                    span_id: Some(SPAN_ID.to_owned()),
                    trace_id: Some(TRACE_ID.to_owned()),
                }],
            }],
        }),
    }
}

fn sum() -> Metric {
    Metric {
        name: "total".to_owned(),
        description: None,
        unit: None,
        data: MetricData::Sum(Sum {
            data_points: vec![NumberDataPoint {
                attributes: Vec::new(),
                start_time_unix_nano: Some(1_000),
                time_unix_nano: 3_000,
                value: Number::AsInt(42),
                exemplars: Vec::new(),
            }],
            aggregation_temporality: AggregationTemporality::Cumulative,
            is_monotonic: true,
        }),
    }
}

fn histogram() -> Metric {
    Metric {
        name: "durations".to_owned(),
        description: None,
        unit: Some("ms".to_owned()),
        data: MetricData::Histogram(Histogram {
            data_points: vec![HistogramDataPoint {
                attributes: Vec::new(),
                start_time_unix_nano: Some(1_000),
                time_unix_nano: 3_000,
                count: 3,
                sum: None,
                bucket_counts: vec![1, 2],
                explicit_bounds: vec![10.0],
                exemplars: Vec::new(),
                min: Some(-5.0),
                max: Some(20.0),
            }],
            aggregation_temporality: AggregationTemporality::Delta,
        }),
    }
}

fn exponential_histogram() -> Metric {
    Metric {
        name: "spread".to_owned(),
        description: None,
        unit: None,
        data: MetricData::ExponentialHistogram(ExponentialHistogram {
            data_points: vec![ExponentialHistogramDataPoint {
                attributes: Vec::new(),
                start_time_unix_nano: Some(1_000),
                time_unix_nano: 3_000,
                count: 6,
                sum: Some(-1.5),
                scale: -2,
                zero_count: 1,
                positive: Buckets {
                    offset: 1,
                    bucket_counts: vec![2, 1],
                },
                negative: Buckets {
                    offset: -1,
                    bucket_counts: vec![2],
                },
                exemplars: Vec::new(),
                min: Some(-4.0),
                max: Some(8.0),
                zero_threshold: 0.001,
            }],
            aggregation_temporality: AggregationTemporality::Cumulative,
        }),
    }
}
