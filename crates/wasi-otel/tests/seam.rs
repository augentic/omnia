//! Seam test for `wasi:otel`: drive the instrumented `otel` example guest and
//! confirm its telemetry crossed the boundary into the host exporter.
//!
//! The guest's handler emits spans and metrics (via both the `tracing` and
//! native `OTel` APIs). Swapping the default no-op exporter for a `CapturingOtel`
//! that counts what it receives lets the test assert the effect host-side —
//! the RFC's "capturing backend" pattern — rather than trusting a `200` alone.
//!
//! The guest is built by `cargo make build-guests`; the test skips locally when
//! it is absent and fails under CI so the pipeline never passes vacuously.

#![cfg(not(target_arch = "wasm32"))]

use std::sync::{Arc, Mutex};

use anyhow::{Context as _, Result};
use futures::FutureExt as _;
use omnia::wasmtime_wasi::ResourceTable;
use omnia::{
    Backend as _, DeploymentBuilder, FutureResult, HasHttp, MountRegistry, Runtime, StoreCtx,
};
use omnia_testkit::{find_guest, http};
use omnia_wasi_http::{HttpDefault, WasiHttp, WasiHttpCtxView};
use omnia_wasi_otel::{HasOtel, WasiOtel, WasiOtelCtx};
use opentelemetry_proto::tonic::collector::metrics::v1::ExportMetricsServiceRequest;
use opentelemetry_proto::tonic::collector::trace::v1::ExportTraceServiceRequest;

/// Spans and metrics observed at the host export boundary.
#[derive(Debug, Default)]
struct Captured {
    spans: usize,
    metrics: usize,
}

/// A `wasi:otel` backend that counts exported spans and metrics instead of
/// discarding them, shared across bundle clones so the test can read the
/// totals after the guest runs.
#[derive(Debug, Clone, Default)]
struct CapturingOtel {
    captured: Arc<Mutex<Captured>>,
}

impl WasiOtelCtx for CapturingOtel {
    fn export_traces(&self, request: ExportTraceServiceRequest) -> FutureResult<()> {
        let spans = request
            .resource_spans
            .iter()
            .flat_map(|rs| &rs.scope_spans)
            .map(|ss| ss.spans.len())
            .sum::<usize>();
        let captured = Arc::clone(&self.captured);
        async move {
            captured.lock().expect("otel capture lock").spans += spans;
            Ok(())
        }
        .boxed()
    }

    fn export_metrics(&self, request: ExportMetricsServiceRequest) -> FutureResult<()> {
        let metrics = request
            .resource_metrics
            .iter()
            .flat_map(|rm| &rm.scope_metrics)
            .map(|sm| sm.metrics.len())
            .sum::<usize>();
        let captured = Arc::clone(&self.captured);
        async move {
            captured.lock().expect("otel capture lock").metrics += metrics;
            Ok(())
        }
        .boxed()
    }
}

/// The `examples/otel` backend bundle, but with the no-op exporter swapped for
/// the capturing one.
#[derive(Clone)]
struct Bundle {
    http: HttpDefault,
    otel: CapturingOtel,
}

impl HasHttp for Bundle {
    fn http_view<'a>(&'a mut self, table: &'a mut ResourceTable) -> WasiHttpCtxView<'a> {
        self.http.as_view(table)
    }
}

impl HasOtel for Bundle {
    fn otel_ctx(&mut self) -> &mut dyn WasiOtelCtx {
        &mut self.otel
    }
}

async fn runtime() -> Result<Option<(Runtime<Bundle>, CapturingOtel)>> {
    let Some(wasm) = find_guest("otel_wasm.wasm", "cargo make build-guests") else {
        return Ok(None);
    };

    let bundle = Bundle {
        http: HttpDefault::connect().await.context("connecting http")?,
        otel: CapturingOtel::default(),
    };
    let exporter = bundle.otel.clone();

    let mut deployment =
        DeploymentBuilder::new().wasm(wasm).build::<StoreCtx<Bundle>>().await.context("build")?;
    deployment.host::<WasiHttp, Bundle>().context("link http")?;
    deployment.host::<WasiOtel, Bundle>().context("link otel")?;
    let registry = deployment.into_registry().context("assemble registry")?;

    let runtime = Runtime::from_parts(
        Arc::new(registry),
        Vec::new(),
        Arc::new(MountRegistry::default()),
        bundle,
    );
    Ok(Some((runtime, exporter)))
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn guest_telemetry() -> Result<()> {
    let Some((runtime, exporter)) = runtime().await? else {
        return Ok(());
    };

    let response = http::post_json(&runtime, "/", r#"{"trace":"me"}"#).await?;
    assert!(response.status().is_success(), "instrumented guest handles the request");

    // Metrics flush deterministically when the guest's telemetry guard drops at
    // the end of the handler, so they are the reliable proof that instrumentation
    // crossed into the host exporter. (Spans ride a separate sampled batch flush,
    // counted here only for diagnostics.)
    let (spans, metrics) = {
        let captured = exporter.captured.lock().expect("otel capture lock");
        (captured.spans, captured.metrics)
    };
    assert!(
        metrics > 0,
        "the guest's metrics reached the host exporter (spans={spans}, metrics={metrics})"
    );

    Ok(())
}
