//! Default no-op implementation for wasi-otel
//!
//! This is a lightweight implementation for development use only.
//! It logs telemetry data but doesn't export it anywhere.
//! For production use, use the `be-opentelemetry` backend.

#![allow(clippy::used_underscore_binding)]

use anyhow::Result;
use futures::FutureExt;
use opentelemetry_proto::tonic::collector::metrics::v1::ExportMetricsServiceRequest;
use opentelemetry_proto::tonic::collector::trace::v1::ExportTraceServiceRequest;
use qwasr::{Backend, FutureResult};
use tracing::instrument;

use crate::host::WasiOtelCtx;

#[derive(Debug, Clone, Default)]
pub struct ConnectOptions;

impl qwasr::FromEnv for ConnectOptions {
    fn from_env() -> Result<Self> {
        Ok(Self)
    }
}

/// Default implementation for `wasi:otel`.
#[derive(Debug, Clone)]
pub struct OtelDefault;

impl Backend for OtelDefault {
    type ConnectOptions = ConnectOptions;

    #[instrument]
    async fn connect_with(_options: Self::ConnectOptions) -> Result<Self> {
        tracing::debug!("Initializing no-op OTel exporter");
        tracing::warn!("Using no-op OTel exporter - will log but not export");
        Ok(Self)
    }
}

impl WasiOtelCtx for OtelDefault {
    /// Log traces but don't export them.
    ///
    /// This is a no-op implementation for development use only.
    fn export_traces(&self, request: ExportTraceServiceRequest) -> FutureResult<()> {
        async move {
            let span_count = request
                .resource_spans
                .iter()
                .map(|rs| rs.scope_spans.iter().map(|ss| ss.spans.len()).sum::<usize>())
                .sum::<usize>();
            tracing::debug!("No-op OTel exporter: would export {span_count} spans");
            Ok(())
        }
        .boxed()
    }

    /// Log metrics but don't export them.
    ///
    /// This is a no-op implementation for development use only.
    fn export_metrics(&self, request: ExportMetricsServiceRequest) -> FutureResult<()> {
        async move {
            let metric_count = request
                .resource_metrics
                .iter()
                .map(|rm| rm.scope_metrics.iter().map(|sm| sm.metrics.len()).sum::<usize>())
                .sum::<usize>();
            tracing::debug!("No-op OTel exporter: would export {metric_count} metrics");
            Ok(())
        }
        .boxed()
    }
}
