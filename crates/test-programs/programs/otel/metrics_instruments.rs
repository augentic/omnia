//! The OpenTelemetry metrics API aggregates in the guest: an instrument of
//! each synchronous kind records through `global::meter`, a mid-run `flush`
//! exports the first collection, and the flush at the end of the run exports
//! only what was recorded after it — a delta counter and a cumulative
//! up-down counter, while the histograms and the gauge, which saw nothing
//! new, stay out. Along the way: attribute sets match whatever order their
//! keys arrive in, a counter drops a negative increment, values land in
//! their explicit buckets (default and custom), and an instrument built with
//! invalid boundaries records nothing.

#![cfg(target_arch = "wasm32")]

use opentelemetry::{KeyValue, global};

omnia_sdk::command!(scenario);

async fn scenario() {
    let meter = global::meter("guest");
    let requests = meter.u64_counter("requests").with_unit("{request}").build();
    let bytes = meter.f64_counter("bytes").build();
    let latency = meter.f64_histogram("latency").build();
    let sizes = meter.u64_histogram("sizes").with_boundaries(vec![10.0, 100.0]).build();
    let broken = meter.f64_histogram("broken").with_boundaries(vec![100.0, 10.0]).build();
    let inflight = meter.i64_up_down_counter("inflight").build();
    let queue_depth = meter.i64_gauge("queue_depth").build();

    // One series per attribute set, however the keys are ordered.
    requests.add(1, &[KeyValue::new("route", "/a"), KeyValue::new("method", "GET")]);
    requests.add(2, &[KeyValue::new("method", "GET"), KeyValue::new("route", "/a")]);
    requests.add(5, &[KeyValue::new("route", "/b"), KeyValue::new("method", "GET")]);
    // A counter only increases: the negative increment is dropped.
    bytes.add(1.5, &[]);
    bytes.add(-4.0, &[]);
    // Default bounds: on the first bound, on an inner bound, inside a bucket,
    // past the last bound.
    latency.record(0.0, &[]);
    latency.record(5.0, &[]);
    latency.record(12.0, &[]);
    latency.record(20000.0, &[]);
    // Custom bounds: on the first bound and just past it.
    sizes.record(10, &[]);
    sizes.record(11, &[]);
    // Descending bounds: never exported.
    broken.record(1.0, &[]);
    inflight.add(3, &[]);
    inflight.add(-1, &[]);
    queue_depth.record(7, &[]);
    queue_depth.record(9, &[]);

    omnia_wasi_otel::flush().await;

    // A second build of an instrument shares its series.
    let again = meter.u64_counter("requests").build();
    again.add(10, &[KeyValue::new("method", "GET"), KeyValue::new("route", "/a")]);
    inflight.add(-1, &[]);
}
