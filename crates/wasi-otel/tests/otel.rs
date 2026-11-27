//! End-to-end tests for `omnia:otel`: every scenario runs a real guest
//! component from `crates/test-programs` through the omnia runtime against an
//! inline backend, which records what it is handed or fails it by name. The
//! suite proves the guest-side telemetry flush delivers spans across the
//! boundary without stalling the export task. The `collector_*` rows run the
//! default backend instead, against an OTLP collector on a loopback port.

#![cfg(not(target_arch = "wasm32"))]

use std::net::SocketAddr;
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use futures::FutureExt as _;
use omnia::{ExitStatus, FutureResult, LevelFilter, Provides, Telemetry, telemetry};
use omnia_test::host::{Deployment, deliver};
use omnia_wasi_messaging::WasiMessaging;
use omnia_wasi_otel::{Error, OtelClient, OtelDefault, WasiOtel};
use omnia_wasi_websocket::{Event, WasiWebSocket, WebSocketHandler};
use opentelemetry_proto::tonic::collector::metrics::v1::metrics_service_server::{
    MetricsService, MetricsServiceServer,
};
use opentelemetry_proto::tonic::collector::metrics::v1::{
    ExportMetricsServiceRequest, ExportMetricsServiceResponse,
};
use opentelemetry_proto::tonic::collector::trace::v1::trace_service_server::{
    TraceService, TraceServiceServer,
};
use opentelemetry_proto::tonic::collector::trace::v1::{
    ExportTraceServiceRequest, ExportTraceServiceResponse,
};
use opentelemetry_proto::tonic::common::v1::any_value::Value as AnyValue;
use opentelemetry_proto::tonic::common::v1::{self as common, ArrayValue, KeyValueList};
use opentelemetry_proto::tonic::metrics::v1::exemplar::Value as ExemplarValue;
use opentelemetry_proto::tonic::metrics::v1::exponential_histogram_data_point::Buckets;
use opentelemetry_proto::tonic::metrics::v1::metric::Data;
use opentelemetry_proto::tonic::metrics::v1::number_data_point::Value;
use opentelemetry_proto::tonic::metrics::v1::{
    AggregationTemporality, HistogramDataPoint, Metric, ResourceMetrics, Sum,
};
use opentelemetry_proto::tonic::trace::v1::span::SpanKind;
use opentelemetry_proto::tonic::trace::v1::status::StatusCode;
use opentelemetry_proto::tonic::trace::v1::{ResourceSpans, Span};
use tokio::net::TcpListener;
use tokio::sync::watch;
use tonic::metadata::MetadataMap;
use tonic::transport::Server;
use tonic::transport::server::TcpIncoming;
use tonic::{Request, Response, Status};

// Every guest program in `crates/test-programs` must have a matching test
// here; a new program without one fails to compile.
test_programs::foreach_otel!();

#[derive(Clone, Debug)]
struct Backends<C>(C);

impl<C: OtelClient + Clone> Provides<WasiOtel> for Backends<C> {
    fn borrow(&mut self) -> &dyn OtelClient {
        &self.0
    }
}

#[derive(Clone, Debug, Default)]
struct Recording {
    traces: Arc<Mutex<Vec<ExportTraceServiceRequest>>>,
    metrics: Arc<Mutex<Vec<ExportMetricsServiceRequest>>>,
}

impl Recording {
    fn span_names(&self) -> Vec<String> {
        self.spans().into_iter().map(|(_, span)| span.name).collect()
    }

    // every exported span with the name of its instrumentation scope
    fn spans(&self) -> Vec<(String, Span)> {
        self.traces
            .lock()
            .expect("traces lock")
            .iter()
            .flat_map(|request| &request.resource_spans)
            .flat_map(|resource| &resource.scope_spans)
            .flat_map(|scope| {
                let name = scope.scope.as_ref().map_or_default(|s| s.name.clone());
                scope.spans.iter().map(move |span| (name.clone(), span.clone()))
            })
            .collect()
    }

    fn metric_exports(&self) -> Vec<Vec<Metric>> {
        self.metrics
            .lock()
            .expect("metrics lock")
            .iter()
            .map(|request| {
                request
                    .resource_metrics
                    .iter()
                    .flat_map(|resource| &resource.scope_metrics)
                    .flat_map(|scope| scope.metrics.clone())
                    .collect()
            })
            .collect()
    }
}

fn attribute<'a>(attributes: &'a [common::KeyValue], key: &str) -> Option<&'a AnyValue> {
    attributes.iter().find(|kv| kv.key == key)?.value.as_ref()?.value.as_ref()
}

fn metric<'a>(metrics: &'a [Metric], name: &str) -> &'a Metric {
    metrics
        .iter()
        .find(|metric| metric.name == name)
        .unwrap_or_else(|| panic!("metric `{name}` exported"))
}

fn sum<'a>(metrics: &'a [Metric], name: &str) -> &'a Sum {
    match &metric(metrics, name).data {
        Some(Data::Sum(sum)) => sum,
        other => panic!("`{name}` must be a sum, got {other:?}"),
    }
}

fn histogram_point<'a>(metrics: &'a [Metric], name: &str) -> &'a HistogramDataPoint {
    let Some(Data::Histogram(histogram)) = &metric(metrics, name).data else {
        panic!("`{name}` must be a histogram");
    };
    assert_eq!(histogram.aggregation_temporality, AggregationTemporality::Delta as i32);
    let [point] = histogram.data_points.as_slice() else {
        panic!("one attribute set on `{name}`, got {}", histogram.data_points.len());
    };
    point
}

impl OtelClient for Recording {
    fn export_traces(&self, request: ExportTraceServiceRequest) -> FutureResult<()> {
        self.traces.lock().expect("traces lock").push(request);
        async { Ok(()) }.boxed()
    }

    fn export_metrics(&self, request: ExportMetricsServiceRequest) -> FutureResult<()> {
        self.metrics.lock().expect("metrics lock").push(request);
        async { Ok(()) }.boxed()
    }
}

async fn run_guest(wasm: &str) -> Recording {
    let recording = Recording::default();
    run(wasm, None, recording.clone()).await;
    recording
}

// guest spans join the runtime's own span for a run or a trigger at `info`,
// so providers install at that level and the suite adds no span of its own
fn install_telemetry() {
    Telemetry::new("otel-e2e").filter("info").build().expect("telemetry installs");
}

// the deadline turns a telemetry-flush deadlock into a failure, not a hung suite
async fn run<C: OtelClient + Clone>(wasm: &str, level: Option<LevelFilter>, client: C) {
    install_telemetry();

    let mut deployment = Deployment::new().guest("guest", wasm);
    if let Some(level) = level {
        deployment = deployment.level(level);
    }

    // linked by hand: `run_host` would add a second `WasiOtel` beside the one under test
    let status = tokio::time::timeout(
        Duration::from_secs(300),
        deployment.run(Backends(client), |deployment| {
            deployment.host::<WasiOtel>()?;
            Ok(())
        }),
    )
    .await
    .expect("guest must not stall on the telemetry flush")
    .expect("guest runs");
    assert_eq!(status, ExitStatus::SUCCESS, "guest `{wasm}` failed");
}

#[tokio::test]
async fn otel_spawned_task_pending() {
    let recording = run_guest(test_programs::OTEL_SPAWNED_TASK_PENDING).await;
    assert_eq!(recording.span_names(), ["traced"]);
}

// The `Handler` bound itself is checked when the guest compiles.
#[tokio::test]
async fn otel_axum_handler() {
    let recording = run_guest(test_programs::OTEL_AXUM_HANDLER).await;
    assert_eq!(recording.span_names(), ["traced"]);
    // no metrics recorded, so the flush skips the export rather than send an empty one
    assert!(recording.metrics.lock().expect("metrics lock").is_empty());
}

// The guest's handlers open no telemetry scope, so their spans arrive only
// through their bindings' scopes.
#[tokio::test]
async fn otel_incoming_handlers() {
    install_telemetry();
    let recording = Recording::default();
    let backends = omnia_test::host::Backends::defaults().await.otel(recording.clone());
    let runtime = Deployment::new()
        .guest("guest", test_programs::OTEL_INCOMING_HANDLERS)
        .boot(backends, |deployment| {
            deployment.host::<WasiMessaging>()?;
            deployment.host::<WasiWebSocket>()?;
            deployment.host::<WasiOtel>()?;
            Ok(())
        })
        .await
        .expect("runtime boots");

    deliver(&runtime, "orders", "order-1").await.expect("the delivery is handled");
    let websocket = WebSocketHandler::new(&runtime)
        .expect("websocket routes consistent")
        .expect("the guest exports the websocket handler");
    let mut event = Event::new(b"ping".to_vec());
    event.socket_addr = Some("peer-1".to_owned());
    websocket.handle(event).await.expect("the event is handled");

    assert_eq!(recording.span_names(), ["messaging", "websocket"]);
    runtime.shutdown();
}

#[tokio::test]
async fn otel_span_tree() {
    let unix_nanos = || {
        let since_epoch = SystemTime::now().duration_since(UNIX_EPOCH).expect("clock past epoch");
        u64::try_from(since_epoch.as_nanos()).expect("nanos fit u64")
    };
    let before = unix_nanos();
    let recording = run_guest(test_programs::OTEL_SPAN_TREE).await;
    let after = unix_nanos();

    // one flush, grouped by scope in first-ended order with the API tracer's
    // schema URL
    let traces = recording.traces.lock().expect("traces lock").clone();
    let [export] = traces.as_slice() else {
        panic!("one flush, got {}", traces.len());
    };
    let [resource] = export.resource_spans.as_slice() else {
        panic!("one resource, got {}", export.resource_spans.len());
    };
    let scopes: Vec<_> = resource
        .scope_spans
        .iter()
        .map(|group| {
            let name = group.scope.as_ref().map_or("", |scope| scope.name.as_str());
            (name, group.schema_url.as_str(), group.spans.len())
        })
        .collect();
    assert_eq!(scopes, [("global", "", 3), ("guest", "https://guest", 1)]);

    let spans = recording.spans();
    let find = |name: &str| {
        spans
            .iter()
            .find(|(_, span)| span.name == name)
            .unwrap_or_else(|| panic!("span `{name}` exported"))
    };
    assert_eq!(spans.len(), 4, "spans: {:?}", recording.span_names());
    let (parent_scope, parent) = find("parent");
    let (child_scope, child) = find("child");
    let (sibling_scope, sibling) = find("sibling");
    let (api_scope, api) = find("api");

    // each span sits in the scope that produced it
    assert_eq!(
        (parent_scope.as_str(), child_scope.as_str(), sibling_scope.as_str(), api_scope.as_str()),
        ("global", "global", "global", "guest")
    );

    // one trace: the root's parent is the host's span, which no guest span
    // is, and the children sit under their guest parent
    assert_eq!(parent.parent_span_id.len(), 8, "the root's parent is the host span");
    assert!(spans.iter().all(|(_, span)| span.span_id != parent.parent_span_id));
    assert_eq!(parent.trace_state, "");
    assert_eq!(child.parent_span_id, parent.span_id);
    assert_eq!(sibling.parent_span_id, parent.span_id);
    assert_eq!(api.parent_span_id, parent.span_id);
    assert!(spans.iter().all(|(_, span)| span.trace_id == parent.trace_id));

    // sampled, under a parent known to be local, and internal
    for (_, span) in &spans {
        assert_eq!((span.flags, span.kind), (0x101, SpanKind::Internal as i32), "`{}`", span.name);
    }

    // every SDK timestamp lands inside the run's wall-clock window
    for span in [parent, child, sibling, api] {
        let (start, end) = (span.start_time_unix_nano, span.end_time_unix_nano);
        assert!(before <= start && start <= end && end <= after, "`{}`: {start}..{end}", span.name);
    }

    // the child's field, event, and the error status the event implies
    assert_eq!(attribute(&child.attributes, "answer"), Some(&AnyValue::IntValue(42)));
    let [event] = child.events.as_slice() else {
        panic!("one event on `child`, got {:?}", child.events);
    };
    assert_eq!(attribute(&event.attributes, "level"), Some(&AnyValue::StringValue("ERROR".into())));
    assert_eq!(child.status.as_ref().map(|s| s.code), Some(StatusCode::Error as i32));
    assert_eq!(parent.status.as_ref().map(|s| s.code), Some(StatusCode::Unset as i32));

    // the API span's attribute, and its link to a remote context with the
    // context's `tracestate`
    assert_eq!(attribute(&api.attributes, "kind"), Some(&AnyValue::StringValue("direct".into())));
    assert_eq!(api.status.as_ref().map(|s| s.code), Some(StatusCode::Unset as i32));
    let [api_link] = api.links.as_slice() else {
        panic!("one link on `api`, got {:?}", api.links);
    };
    assert_eq!((api_link.flags, api_link.trace_state.as_str()), (0x301, "vendor=opaque,other=a:b"));
}

#[tokio::test]
async fn otel_raw_export() {
    let recording = run_guest(test_programs::OTEL_RAW_EXPORT).await;

    // the empty exports reached no backend, and the host's resource leads
    // each request
    let traces = recording.traces.lock().expect("traces lock").clone();
    let [traces] = traces.as_slice() else {
        panic!("one span export, got {}", traces.len());
    };
    let metrics = recording.metrics.lock().expect("metrics lock").clone();
    let [metrics] = metrics.as_slice() else {
        panic!("one metrics export, got {}", metrics.len());
    };
    let [resource_spans] = traces.resource_spans.as_slice() else {
        panic!("one resource, got {}", traces.resource_spans.len());
    };
    let [resource_metrics] = metrics.resource_metrics.as_slice() else {
        panic!("one resource, got {}", metrics.resource_metrics.len());
    };
    for resource in [&resource_spans.resource, &resource_metrics.resource] {
        let attributes = &resource.as_ref().expect("the host's resource").attributes;
        let service = AnyValue::StringValue("otel-e2e".into());
        assert_eq!(attribute(attributes, "service.name"), Some(&service));
    }

    raw_spans(resource_spans);
    raw_metrics(resource_metrics);
}

// the span `raw_export` builds, as it built it
fn raw_spans(resource: &ResourceSpans) {
    // the scope as given
    let [group] = resource.scope_spans.as_slice() else {
        panic!("one scope, got {}", resource.scope_spans.len());
    };
    let scope = group.scope.as_ref().expect("a scope");
    assert_eq!(
        (scope.name.as_str(), scope.version.as_str(), scope.dropped_attributes_count),
        ("raw", "1.2.3", 2)
    );
    assert_eq!(
        attribute(&scope.attributes, "scope.attr"),
        Some(&AnyValue::StringValue("x".into()))
    );
    assert_eq!(group.schema_url, "https://raw");

    // the ids as bytes, and the flags with the remote parent's bits
    let [span] = group.spans.as_slice() else {
        panic!("one span, got {}", group.spans.len());
    };
    assert_eq!(span.trace_id, (0x01..=0x10).collect::<Vec<u8>>());
    assert_eq!(span.span_id, (0x11..=0x18).collect::<Vec<u8>>());
    assert_eq!(span.parent_span_id, (0x21..=0x28).collect::<Vec<u8>>());
    assert_eq!((span.trace_state.as_str(), span.flags), ("vendor=opaque", 0x303));
    assert_eq!(span.kind, SpanKind::Server as i32);
    assert_eq!((span.start_time_unix_nano, span.end_time_unix_nano), (1_000, 2_000));

    // every value case, with the JSON read into maps and arrays
    let any = |value| common::AnyValue { value: Some(value) };
    let array = |values| AnyValue::ArrayValue(ArrayValue { values });
    let map = |key: &str, value| {
        AnyValue::KvlistValue(KeyValueList {
            values: vec![common::KeyValue {
                key: key.to_owned(),
                value: Some(value),
                ..common::KeyValue::default()
            }],
        })
    };
    let json = map(
        "a",
        any(array(vec![
            any(AnyValue::IntValue(1)),
            any(AnyValue::StringValue("b".into())),
            common::AnyValue { value: None },
            any(map("c", any(AnyValue::DoubleValue(2.5)))),
        ])),
    );
    let values: Vec<_> =
        span.attributes.iter().map(|kv| (kv.key.as_str(), kv.value.clone())).collect();
    assert_eq!(
        values,
        [
            ("string", Some(any(AnyValue::StringValue("text".into())))),
            ("bool", Some(any(AnyValue::BoolValue(true)))),
            ("int", Some(any(AnyValue::IntValue(-7)))),
            ("double", Some(any(AnyValue::DoubleValue(2.5)))),
            ("bytes", Some(any(AnyValue::BytesValue(vec![0, 255])))),
            (
                "strings",
                Some(any(array(vec![
                    any(AnyValue::StringValue("a".into())),
                    any(AnyValue::StringValue("b".into())),
                ]))),
            ),
            (
                "bools",
                Some(any(array(vec![
                    any(AnyValue::BoolValue(true)),
                    any(AnyValue::BoolValue(false))
                ]))),
            ),
            (
                "ints",
                Some(any(array(vec![any(AnyValue::IntValue(1)), any(AnyValue::IntValue(2))])))
            ),
            ("doubles", Some(any(array(vec![any(AnyValue::DoubleValue(0.5))])))),
            ("json", Some(any(json))),
            ("empty", Some(common::AnyValue { value: None })),
        ]
    );

    // the dropped counts, a link whose remoteness is unknown, and an error
    // with no description
    assert_eq!(
        (span.dropped_attributes_count, span.dropped_events_count, span.dropped_links_count),
        (3, 4, 5)
    );
    let [event] = span.events.as_slice() else {
        panic!("one event, got {:?}", span.events);
    };
    assert_eq!(
        (event.name.as_str(), event.time_unix_nano, event.dropped_attributes_count),
        ("tick", 1_500, 1)
    );
    let [link] = span.links.as_slice() else {
        panic!("one link, got {:?}", span.links);
    };
    assert_eq!(link.span_id, (0x41..=0x48).collect::<Vec<u8>>());
    assert_eq!((link.flags, link.trace_state.as_str()), (0x01, ""));
    let status = span.status.as_ref().expect("a status");
    assert_eq!((status.code, status.message.as_str()), (StatusCode::Error as i32, ""));
}

// the metrics `raw_export` builds, as it built them
fn raw_metrics(resource: &ResourceMetrics) {
    // no scope, and an unset description or unit read as empty
    let [group] = resource.scope_metrics.as_slice() else {
        panic!("one scope, got {}", resource.scope_metrics.len());
    };
    assert_eq!((group.scope.as_ref(), group.schema_url.as_str()), (None, ""));
    let metrics = &group.metrics;
    let labels: Vec<_> = metrics
        .iter()
        .map(|metric| (metric.name.as_str(), metric.description.as_str(), metric.unit.as_str()))
        .collect();
    assert_eq!(
        labels,
        [
            ("temperature", "raw metrics", ""),
            ("total", "", ""),
            ("durations", "", "ms"),
            ("spread", "", ""),
        ]
    );

    // a gauge with no start time, and its exemplar's ids as bytes
    let Some(Data::Gauge(gauge)) = &metric(metrics, "temperature").data else {
        panic!("`temperature` must be a gauge");
    };
    let [point] = gauge.data_points.as_slice() else {
        panic!("one point, got {}", gauge.data_points.len());
    };
    assert_eq!(
        (point.start_time_unix_nano, point.time_unix_nano, point.value),
        (0, 3_000, Some(Value::AsDouble(21.5)))
    );
    let [exemplar] = point.exemplars.as_slice() else {
        panic!("one exemplar, got {}", point.exemplars.len());
    };
    assert_eq!(exemplar.span_id, (0x11..=0x18).collect::<Vec<u8>>());
    assert_eq!(exemplar.trace_id, (0x01..=0x10).collect::<Vec<u8>>());
    assert_eq!(
        (exemplar.time_unix_nano, exemplar.value),
        (2_900, Some(ExemplarValue::AsDouble(21.4)))
    );
    let probe = AnyValue::StringValue("p1".into());
    assert_eq!(attribute(&exemplar.filtered_attributes, "probe"), Some(&probe));

    // a cumulative sum
    let total = sum(metrics, "total");
    assert_eq!(
        (total.aggregation_temporality, total.is_monotonic),
        (AggregationTemporality::Cumulative as i32, true)
    );
    let [point] = total.data_points.as_slice() else {
        panic!("one point, got {}", total.data_points.len());
    };
    assert_eq!((point.start_time_unix_nano, point.value), (1_000, Some(Value::AsInt(42))));

    // a histogram with no sum
    let durations = histogram_point(metrics, "durations");
    assert_eq!(
        (durations.count, durations.sum, durations.min, durations.max),
        (3, None, Some(-5.0), Some(20.0))
    );
    assert_eq!(
        (durations.bucket_counts.as_slice(), durations.explicit_bounds.as_slice()),
        ([1, 2].as_slice(), [10.0].as_slice())
    );

    // an exponential histogram with a negative scale and both bucket runs
    let Some(Data::ExponentialHistogram(spread)) = &metric(metrics, "spread").data else {
        panic!("`spread` must be an exponential histogram");
    };
    assert_eq!(spread.aggregation_temporality, AggregationTemporality::Cumulative as i32);
    let [point] = spread.data_points.as_slice() else {
        panic!("one point, got {}", spread.data_points.len());
    };
    assert_eq!(
        (point.count, point.sum, point.scale, point.zero_count, point.zero_threshold),
        (6, Some(-1.5), -2, 1, 0.001)
    );
    assert_eq!((point.min, point.max), (Some(-4.0), Some(8.0)));
    let counts = |buckets: &Option<Buckets>| {
        buckets.as_ref().map(|buckets| (buckets.offset, buckets.bucket_counts.clone()))
    };
    assert_eq!(counts(&point.positive), Some((1, vec![2, 1])));
    assert_eq!(counts(&point.negative), Some((-1, vec![2])));
}

// A server's default level, above the guest's `info` metric events.
#[tokio::test]
async fn otel_metrics_instruments() {
    let recording = Recording::default();
    run(test_programs::OTEL_METRICS_INSTRUMENTS, Some(LevelFilter::WARN), recording.clone()).await;

    let exports = recording.metric_exports();
    let [first, second] = exports.as_slice() else {
        panic!("the mid-run flush and the final one export, got {}", exports.len());
    };
    let names = |metrics: &[Metric]| {
        let mut names: Vec<_> = metrics.iter().map(|m| m.name.clone()).collect();
        names.sort();
        names
    };
    // `broken` (descending bounds) records nothing, so it never exports.
    assert_eq!(
        names(first),
        ["big", "bytes", "hits", "inflight", "latency", "queue_depth", "requests", "sizes"]
    );
    // Only what was recorded since the first flush: no histogram, no gauge.
    assert_eq!(names(second), ["inflight", "requests"]);

    // A `monotonic_counter.*` tracing field is a counter of its own.
    let hits = sum(first, "hits");
    assert!(hits.is_monotonic);
    assert_eq!(hits.data_points.len(), 1, "both increments land on one data point");
    assert_eq!(hits.data_points[0].value, Some(Value::AsInt(2)));

    // Counter: delta, one data point per attribute set however its keys were
    // ordered, and the later increment alone in the second export (through a
    // second build of the instrument).
    assert_eq!(metric(first, "requests").unit, "{request}");
    let requests = sum(first, "requests");
    assert!(requests.is_monotonic);
    assert_eq!(requests.aggregation_temporality, AggregationTemporality::Delta as i32);
    let by_route = |sum: &Sum, route: &str| {
        let route = AnyValue::StringValue(route.into());
        let point = sum
            .data_points
            .iter()
            .find(|dp| attribute(&dp.attributes, "route") == Some(&route))
            .unwrap_or_else(|| panic!("a data point for {route:?}"));
        let keys: Vec<_> = point.attributes.iter().map(|kv| kv.key.as_str()).collect();
        assert_eq!(keys, ["method", "route"], "an attribute set exports sorted by key");
        point.value
    };
    assert_eq!(requests.data_points.len(), 2);
    assert_eq!(by_route(requests, "/a"), Some(Value::AsInt(3)));
    assert_eq!(by_route(requests, "/b"), Some(Value::AsInt(5)));
    let later = sum(second, "requests");
    assert_eq!(later.data_points.len(), 1);
    assert_eq!(by_route(later, "/a"), Some(Value::AsInt(10)));
    assert_eq!(
        later.data_points[0].start_time_unix_nano, requests.data_points[0].time_unix_nano,
        "a delta interval starts where the previous one ended"
    );

    // An f64 counter carries a double and drops its negative increment.
    let bytes = sum(first, "bytes");
    assert!(bytes.is_monotonic);
    assert_eq!(bytes.data_points.len(), 1);
    assert_eq!(bytes.data_points[0].value, Some(Value::AsDouble(1.5)));

    // A u64 past the signed range saturates.
    let big = sum(first, "big");
    assert_eq!(big.data_points.len(), 1);
    assert_eq!(big.data_points[0].value, Some(Value::AsInt(i64::MAX)));

    // Up-down counter: cumulative, so the second export carries the running
    // total.
    let updown = |metrics: &[Metric]| {
        let inflight = sum(metrics, "inflight");
        assert!(!inflight.is_monotonic);
        assert_eq!(inflight.aggregation_temporality, AggregationTemporality::Cumulative as i32);
        assert_eq!(inflight.data_points.len(), 1);
        inflight.data_points[0].value
    };
    assert_eq!(updown(first), Some(Value::AsInt(2)));
    assert_eq!(updown(second), Some(Value::AsInt(1)));

    // Gauge: the last value recorded.
    let Some(Data::Gauge(gauge)) = &metric(first, "queue_depth").data else {
        panic!("`queue_depth` must be a gauge");
    };
    assert_eq!(gauge.data_points.len(), 1);
    assert_eq!(gauge.data_points[0].value, Some(Value::AsInt(9)));

    // Histogram over the default bounds: a value on a bound belongs to the
    // bucket it closes, and the last bucket is open-ended.
    let latency = histogram_point(first, "latency");
    assert_eq!(latency.explicit_bounds.len(), 15);
    assert_eq!((latency.count, latency.sum), (4, Some(20017.0)));
    assert_eq!((latency.min, latency.max), (Some(0.0), Some(20000.0)));
    // 0 → (-inf, 0], 5 → (0, 5], 12 → (10, 25], 20000 → (10000, +inf).
    let mut buckets = vec![0; 16];
    for index in [0, 1, 3, 15] {
        buckets[index] = 1;
    }
    assert_eq!(latency.bucket_counts, buckets);

    // Custom bounds over an integer histogram.
    let sizes = histogram_point(first, "sizes");
    assert_eq!(sizes.explicit_bounds, [10.0, 100.0]);
    assert_eq!(sizes.bucket_counts, [1, 1, 0]);
    assert_eq!((sizes.count, sizes.sum), (2, Some(21.0)));
    assert_eq!((sizes.min, sizes.max), (Some(10.0), Some(11.0)));
}

// fails an export with the kind its first span or metric is named for, and
// records each name so the run's exchanges are pinned
#[derive(Clone, Debug, Default)]
struct Faulting {
    exported: Arc<Mutex<Vec<String>>>,
}

impl Faulting {
    fn fail(&self, name: String) -> FutureResult<()> {
        let fault = match name.as_str() {
            "not-found" => Error::NotFound("not found in the suite".to_owned()).into(),
            "denied" => Error::Denied("denied by the suite".to_owned()).into(),
            "invalid" => Error::Invalid("invalid in the suite".to_owned()).into(),
            "unsupported" => Error::Unsupported("unsupported by the suite".to_owned()).into(),
            "unavailable" => Error::Unavailable("unavailable in the suite".to_owned()).into(),
            "other" => anyhow::anyhow!("an untyped backend failure"),
            _ => anyhow::anyhow!("an unscripted export of `{name}`"),
        };
        self.exported.lock().expect("exported names").push(name);
        async move { Err(fault) }.boxed()
    }
}

impl OtelClient for Faulting {
    fn export_traces(&self, request: ExportTraceServiceRequest) -> FutureResult<()> {
        let first = request
            .resource_spans
            .iter()
            .flat_map(|resource| &resource.scope_spans)
            .flat_map(|scope| &scope.spans)
            .next();
        self.fail(first.map_or_default(|span| span.name.clone()))
    }

    fn export_metrics(&self, request: ExportMetricsServiceRequest) -> FutureResult<()> {
        let first = request
            .resource_metrics
            .iter()
            .flat_map(|resource| &resource.scope_metrics)
            .flat_map(|scope| &scope.metrics)
            .next();
        self.fail(first.map_or_default(|metric| metric.name.clone()))
    }
}

#[tokio::test]
async fn otel_error_codes() {
    let faulting = Faulting::default();

    run(test_programs::OTEL_ERROR_CODES, None, faulting.clone()).await;

    // the telemetry the host refused never reached the backend
    let exported = faulting.exported.lock().expect("exported names").clone();
    assert_eq!(
        exported,
        ["not-found", "denied", "invalid", "unsupported", "unavailable", "other", "unavailable"]
    );
}

// An OTLP collector on a loopback port, serving until the test's runtime ends.
// It records each export with its metadata as it arrives, the host's own
// exports included, and answers only once its gate opens.
#[derive(Clone)]
struct Collector {
    traces: Arc<Mutex<Vec<(MetadataMap, ExportTraceServiceRequest)>>>,
    metrics: Arc<Mutex<Vec<(MetadataMap, ExportMetricsServiceRequest)>>>,
    gate: watch::Receiver<bool>,
}

impl Collector {
    async fn serve() -> (Self, SocketAddr, watch::Sender<bool>) {
        let listener = TcpListener::bind("127.0.0.1:0").await.expect("binding a loopback port");
        let address = listener.local_addr().expect("the port's address");
        let (open, gate) = watch::channel(false);
        let collector = Self {
            traces: Arc::default(),
            metrics: Arc::default(),
            gate,
        };
        tokio::spawn(
            Server::builder()
                .add_service(TraceServiceServer::new(collector.clone()))
                .add_service(MetricsServiceServer::new(collector.clone()))
                .serve_with_incoming(TcpIncoming::from(listener)),
        );
        (collector, address, open)
    }

    async fn answer<T: Default>(&self) -> Result<Response<T>, Status> {
        let mut gate = self.gate.clone();
        gate.wait_for(|open| *open).await.map_err(|_closed| Status::unavailable("gate dropped"))?;
        Ok(Response::new(T::default()))
    }

    // the span exports carrying a span named `name`
    fn traces_with(&self, name: &str) -> Vec<(MetadataMap, ExportTraceServiceRequest)> {
        self.traces
            .lock()
            .expect("traces lock")
            .iter()
            .filter(|(_, export)| {
                export
                    .resource_spans
                    .iter()
                    .flat_map(|resource| &resource.scope_spans)
                    .flat_map(|scope| &scope.spans)
                    .any(|span| span.name == name)
            })
            .cloned()
            .collect()
    }

    // the metric exports carrying a metric named `name`
    fn metrics_with(&self, name: &str) -> Vec<(MetadataMap, ExportMetricsServiceRequest)> {
        self.metrics
            .lock()
            .expect("metrics lock")
            .iter()
            .filter(|(_, export)| {
                export
                    .resource_metrics
                    .iter()
                    .flat_map(|resource| &resource.scope_metrics)
                    .flat_map(|scope| &scope.metrics)
                    .any(|metric| metric.name == name)
            })
            .cloned()
            .collect()
    }
}

#[tonic::async_trait]
impl TraceService for Collector {
    async fn export(
        &self, request: Request<ExportTraceServiceRequest>,
    ) -> Result<Response<ExportTraceServiceResponse>, Status> {
        let (metadata, _, export) = request.into_parts();
        self.traces.lock().expect("traces lock").push((metadata, export));
        self.answer().await
    }
}

#[tonic::async_trait]
impl MetricsService for Collector {
    async fn export(
        &self, request: Request<ExportMetricsServiceRequest>,
    ) -> Result<Response<ExportMetricsServiceResponse>, Status> {
        let (metadata, _, export) = request.into_parts();
        self.metrics.lock().expect("metrics lock").push((metadata, export));
        self.answer().await
    }
}

fn collector_telemetry(address: SocketAddr) {
    Telemetry::new("otel-e2e")
        .filter("info")
        .endpoint(format!("http://{address}"))
        .build()
        .expect("telemetry installs");
}

// the flush blocks, so it runs beside the runtime serving the collector
async fn flush_within(bound: Duration) {
    tokio::time::timeout(bound, tokio::task::spawn_blocking(telemetry::flush))
        .await
        .expect("the flush ends within its bound")
        .expect("the flush completes");
}

// The guest runs while the collector holds every answer, under a timeout that
// outlasts the run's deadline, so an export the guest waited on fails the run.
#[tokio::test]
async fn collector_withheld() {
    // SAFETY: nextest runs each test in a process of its own, so no other
    // thread reads the environment while it is written.
    #[expect(
        unsafe_code,
        reason = "the export's headers and timeout are the process environment's"
    )]
    unsafe {
        std::env::set_var("OTEL_EXPORTER_OTLP_HEADERS", "x-omnia-suite=withheld");
        std::env::set_var("OTEL_EXPORTER_OTLP_TIMEOUT", "600000");
    }
    let (collector, address, open) = Collector::serve().await;
    collector_telemetry(address);

    run(test_programs::OTEL_RAW_EXPORT, None, OtelDefault).await;
    open.send(true).expect("the collector holds the gate");
    flush_within(Duration::from_secs(60)).await;

    // the guest's export of each signal, apart from the host's own
    let traces = collector.traces_with("raw");
    let [(span_metadata, traces)] = traces.as_slice() else {
        panic!("one guest span export, got {}", traces.len());
    };
    let metrics = collector.metrics_with("temperature");
    let [(metric_metadata, metrics)] = metrics.as_slice() else {
        panic!("one guest metric export, got {}", metrics.len());
    };
    for metadata in [span_metadata, metric_metadata] {
        let header = metadata.get("x-omnia-suite").and_then(|value| value.to_str().ok());
        assert_eq!(header, Some("withheld"));
    }

    let [resource_spans] = traces.resource_spans.as_slice() else {
        panic!("one resource, got {}", traces.resource_spans.len());
    };
    let [resource_metrics] = metrics.resource_metrics.as_slice() else {
        panic!("one resource, got {}", metrics.resource_metrics.len());
    };
    for resource in [&resource_spans.resource, &resource_metrics.resource] {
        let attributes = &resource.as_ref().expect("the host's resource").attributes;
        let service = AnyValue::StringValue("otel-e2e".into());
        assert_eq!(attribute(attributes, "service.name"), Some(&service));
    }

    let spans: Vec<_> = resource_spans
        .scope_spans
        .iter()
        .flat_map(|scope| &scope.spans)
        .map(|span| span.name.as_str())
        .collect();
    assert_eq!(spans, ["raw"]);
    let metrics: Vec<_> = resource_metrics
        .scope_metrics
        .iter()
        .flat_map(|scope| &scope.metrics)
        .map(|metric| metric.name.as_str())
        .collect();
    assert_eq!(metrics, ["temperature", "total", "durations", "spread"]);
}

// Nothing listens on the port, so every export fails. The host's two
// exporters each retry for up to the timeout, one after the other, which the
// flush's bound allows; the default timeout is ten seconds.
#[tokio::test]
async fn collector_down() {
    const TIMEOUT: Duration = Duration::from_secs(1);

    // SAFETY: nextest runs each test in a process of its own, so no other
    // thread reads the environment while it is written.
    #[expect(unsafe_code, reason = "the export's timeout is the process environment's")]
    unsafe {
        std::env::set_var("OTEL_EXPORTER_OTLP_TIMEOUT", TIMEOUT.as_millis().to_string());
    }
    let closed = std::net::TcpListener::bind("127.0.0.1:0")
        .and_then(|listener| listener.local_addr())
        .expect("a loopback port");
    collector_telemetry(closed);

    run(test_programs::OTEL_RAW_EXPORT, None, OtelDefault).await;
    flush_within(5 * TIMEOUT).await;
}
