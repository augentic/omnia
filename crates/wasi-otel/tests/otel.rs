//! End-to-end tests for `omnia:otel`: every scenario runs a real guest
//! component from `crates/test-programs` through the omnia runtime against an
//! inline recording backend. The suite proves the guest-side telemetry flush
//! delivers spans across the boundary without stalling the export task.

#![cfg(not(target_arch = "wasm32"))]

use std::sync::{Arc, Mutex};
use std::time::Duration;

use futures::FutureExt as _;
use omnia::{ExitStatus, FutureResult, Provides, Telemetry};
use omnia_test::host::Deployment;
use omnia_wasi_otel::{WasiOtel, WasiOtelCtx};
use opentelemetry_proto::tonic::collector::metrics::v1::ExportMetricsServiceRequest;
use opentelemetry_proto::tonic::collector::trace::v1::ExportTraceServiceRequest;
use opentelemetry_proto::tonic::common::v1::any_value::Value as AnyValue;
use opentelemetry_proto::tonic::metrics::v1::metric::Data;
use opentelemetry_proto::tonic::metrics::v1::number_data_point::Value;
use opentelemetry_proto::tonic::metrics::v1::{
    AggregationTemporality, HistogramDataPoint, Metric, Sum,
};
use opentelemetry_proto::tonic::trace::v1::Span;
use opentelemetry_proto::tonic::trace::v1::status::StatusCode;
use tracing::Instrument as _;

// Every guest program in `crates/test-programs` must have a matching test
// here; a new program without one fails to compile.
test_programs::foreach_otel!();

/// The store's backend bundle: just the otel backend under test.
#[derive(Clone, Debug)]
struct Backends(Recording);

impl Provides<WasiOtel> for Backends {
    fn borrow(&mut self) -> &mut dyn WasiOtelCtx {
        &mut self.0
    }
}

/// Records every export it receives, for host-side assertions.
#[derive(Clone, Debug, Default)]
struct Recording {
    traces: Arc<Mutex<Vec<ExportTraceServiceRequest>>>,
    metrics: Arc<Mutex<Vec<ExportMetricsServiceRequest>>>,
}

impl Recording {
    fn span_names(&self) -> Vec<String> {
        self.spans().into_iter().map(|(_, span)| span.name).collect()
    }

    /// Every exported span with the name of its instrumentation scope.
    fn spans(&self) -> Vec<(String, Span)> {
        self.traces
            .lock()
            .expect("traces lock")
            .iter()
            .flat_map(|request| &request.resource_spans)
            .flat_map(|resource| &resource.scope_spans)
            .flat_map(|scope| {
                let name = scope.scope.as_ref().map(|s| s.name.clone()).unwrap_or_default();
                scope.spans.iter().map(move |span| (name.clone(), span.clone()))
            })
            .collect()
    }

    /// The metrics of each export, in export order.
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

fn attribute<'a>(
    attributes: &'a [opentelemetry_proto::tonic::common::v1::KeyValue], key: &str,
) -> Option<&'a AnyValue> {
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

/// The single data point of a delta histogram.
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

impl WasiOtelCtx for Recording {
    fn export_traces(&self, request: ExportTraceServiceRequest) -> FutureResult<()> {
        self.traces.lock().expect("traces lock").push(request);
        async { Ok(()) }.boxed()
    }

    fn export_metrics(&self, request: ExportMetricsServiceRequest) -> FutureResult<()> {
        self.metrics.lock().expect("metrics lock").push(request);
        async { Ok(()) }.boxed()
    }
}

/// Run one guest program against a fresh recording backend.
async fn run_guest(wasm: &str) -> Recording {
    run(Deployment::new().guest("guest", wasm), wasm).await
}

/// Run `deployment` against a fresh recording backend; the deadline turns a
/// telemetry-flush deadlock into a failure instead of a hung suite.
async fn run(deployment: Deployment, label: &str) -> Recording {
    // Guest telemetry grafts onto the host trace: the host-side `export`
    // impls skip unless host telemetry is initialized and a host span is
    // live, so install providers and drive the guest inside a span.
    Telemetry::new("otel-e2e").filter("info").build().expect("telemetry installs");

    let recording = Recording::default();
    // Linked by hand: `run_host` would add a second `WasiOtel` beside the
    // one under test.
    let status = tokio::time::timeout(
        Duration::from_secs(300),
        deployment
            .run(Backends(recording.clone()), |deployment| {
                deployment.host::<WasiOtel, Backends>()?;
                Ok(())
            })
            .instrument(tracing::info_span!("test-drive")),
    )
    .await
    .expect("guest must not stall on the telemetry flush")
    .expect("guest runs");
    assert_eq!(status, ExitStatus::SUCCESS, "guest `{label}` failed");
    recording
}

#[tokio::test]
async fn otel_instrumented_handler() {
    let recording = run_guest(test_programs::OTEL_INSTRUMENTED_HANDLER).await;
    assert_eq!(recording.span_names(), ["traced"]);
    // The scenario records no metrics, so the flush skips the metrics export
    // rather than sending an empty collection.
    assert!(recording.metrics.lock().expect("metrics lock").is_empty());
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
}

#[tokio::test]
async fn otel_span_tree() {
    let recording = run_guest(test_programs::OTEL_SPAN_TREE).await;

    let spans = recording.spans();
    let find = |name: &str| {
        spans
            .iter()
            .find(|(_, span)| span.name == name)
            .unwrap_or_else(|| panic!("span `{name}` exported"))
    };
    assert_eq!(spans.len(), 3, "spans: {:?}", recording.span_names());
    let (parent_scope, parent) = find("parent");
    let (child_scope, child) = find("child");
    let (api_scope, api) = find("api");

    // The `tracing` bridge's spans sit in the layer's scope; the API span in
    // the tracer's own.
    assert_eq!(
        (parent_scope.as_str(), child_scope.as_str(), api_scope.as_str()),
        ("global", "global", "guest")
    );

    // One trace: the host's, which re-parents the guest's root to its live
    // span and leaves the children under their guest parent.
    assert!(!parent.parent_span_id.is_empty(), "the root is grafted onto the host span");
    assert_ne!(parent.parent_span_id, parent.span_id);
    assert_eq!(child.parent_span_id, parent.span_id);
    assert_eq!(api.parent_span_id, parent.span_id);
    assert!(spans.iter().all(|(_, span)| span.trace_id == parent.trace_id));
    assert!(
        child.start_time_unix_nano > 0 && child.end_time_unix_nano >= child.start_time_unix_nano
    );

    // The child's field, event, and the error status the event implies.
    assert_eq!(attribute(&child.attributes, "answer"), Some(&AnyValue::IntValue(42)));
    let [event] = child.events.as_slice() else {
        panic!("one event on `child`, got {:?}", child.events);
    };
    assert_eq!(attribute(&event.attributes, "level"), Some(&AnyValue::StringValue("ERROR".into())));
    assert_eq!(child.status.as_ref().map(|s| s.code), Some(StatusCode::Error as i32));
    assert_eq!(parent.status.as_ref().map(|s| s.code), Some(StatusCode::Unset as i32));

    // The API span's attribute set through `TraceContextExt`.
    assert_eq!(attribute(&api.attributes, "kind"), Some(&AnyValue::StringValue("direct".into())));
    assert_eq!(api.status.as_ref().map(|s| s.code), Some(StatusCode::Unset as i32));
}

#[tokio::test]
async fn otel_metrics_counter() {
    let recording = run_guest(test_programs::OTEL_METRICS_COUNTER).await;

    let exports = recording.metric_exports();
    let [metrics] = exports.as_slice() else {
        panic!("one flush exports one metrics collection, got {}", exports.len());
    };
    let hits = sum(metrics, "hits");
    assert!(hits.is_monotonic);
    assert_eq!(hits.data_points.len(), 1, "both increments land on one data point");
    assert_eq!(hits.data_points[0].value, Some(Value::AsInt(2)));
}

#[tokio::test]
async fn otel_metrics_instruments() {
    let recording = run_guest(test_programs::OTEL_METRICS_INSTRUMENTS).await;

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
    assert_eq!(names(first), ["bytes", "inflight", "latency", "queue_depth", "requests", "sizes"]);
    // Only what was recorded since the first flush: no histogram, no gauge.
    assert_eq!(names(second), ["inflight", "requests"]);

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
