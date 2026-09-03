//! End-to-end tests for `omnia:otel`: every scenario runs a real guest
//! component from `crates/test-programs` through the omnia runtime against an
//! inline recording backend. The suite proves the guest-side telemetry flush
//! delivers spans across the boundary without stalling the export task.

#![cfg(not(target_arch = "wasm32"))]

use std::sync::{Arc, Mutex};
use std::time::Duration;

use futures::FutureExt as _;
use omnia::{ExitStatus, FutureResult, LogMode, Provides, Telemetry};
use omnia_wasi_otel::{WasiOtel, WasiOtelCtx};
use opentelemetry_proto::tonic::collector::metrics::v1::ExportMetricsServiceRequest;
use opentelemetry_proto::tonic::collector::trace::v1::ExportTraceServiceRequest;
use tracing::Instrument as _;

// Every guest program in `crates/test-programs` must have a matching test
// here; a new program without one fails to compile.
test_utils::foreach_otel!();

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
        self.traces
            .lock()
            .expect("traces lock")
            .iter()
            .flat_map(|request| &request.resource_spans)
            .flat_map(|resource| &resource.scope_spans)
            .flat_map(|scope| &scope.spans)
            .map(|span| span.name.clone())
            .collect()
    }
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

/// Run one guest program against a fresh recording backend; the deadline
/// turns a telemetry-flush deadlock into a failure instead of a hung suite.
async fn run_guest(wasm: &str) -> Recording {
    // Guest telemetry grafts onto the host trace: the host-side `export`
    // impls skip unless host telemetry is initialized and a host span is
    // live, so install providers and drive the guest inside a span.
    Telemetry::new("otel-e2e").log_mode(LogMode::Progress).build().expect("telemetry installs");

    let recording = Recording::default();
    // Linked by hand: `run_host` would add a second `WasiOtel` beside the
    // one under test.
    let status = tokio::time::timeout(
        Duration::from_secs(300),
        test_utils::run_command(wasm, vec![], Backends(recording.clone()), |deployment| {
            deployment.host::<WasiOtel, Backends>()?;
            Ok(())
        })
        .instrument(tracing::info_span!("test-drive")),
    )
    .await
    .expect("guest must not stall on the telemetry flush")
    .expect("guest runs");
    assert_eq!(status, ExitStatus::SUCCESS, "guest `{wasm}` failed");
    recording
}

#[tokio::test]
async fn otel_instrumented_handler() {
    let recording = run_guest(test_utils::OTEL_INSTRUMENTED_HANDLER).await;
    assert_eq!(recording.span_names(), ["traced"]);
    // The scenario records no metrics, so the flush skips the metrics export
    // rather than sending an empty collection.
    assert!(recording.metrics.lock().expect("metrics lock").is_empty());
}

#[tokio::test]
async fn otel_spawned_task_pending() {
    let recording = run_guest(test_utils::OTEL_SPAWNED_TASK_PENDING).await;
    assert_eq!(recording.span_names(), ["traced"]);
}
