//! OpenTelemetry exporter service contract and default implementation.

#[cfg(not(target_arch = "wasm32"))]
mod host {
    use std::fmt::Debug;

    use anyhow::Result;
    use futures::FutureExt as _;
    use opentelemetry_proto::tonic::collector::metrics::v1::ExportMetricsServiceRequest;
    use opentelemetry_proto::tonic::collector::trace::v1::ExportTraceServiceRequest;
    use tracing::instrument;

    use crate::{Connect, FutureResult, NoOptions};

    /// Service contract for `omnia:otel` backends.
    pub trait OtelClient: Debug + Send + Sync + 'static {
        /// Export traces using gRPC.
        fn export_traces(&self, request: ExportTraceServiceRequest) -> FutureResult<()>;

        /// Export metrics using gRPC.
        fn export_metrics(&self, request: ExportMetricsServiceRequest) -> FutureResult<()>;
    }

    /// Default no-op implementation of [`OtelClient`].
    #[derive(Debug, Clone, Default)]
    pub struct OtelDefault;

    impl Connect for OtelDefault {
        type ConnectOptions = NoOptions;

        #[instrument(skip_all)]
        async fn connect_with(_options: Self::ConnectOptions) -> Result<Self> {
            tracing::debug!("Initializing no-op OTel exporter");
            tracing::warn!("Using no-op OTel exporter - will log but not export");
            Ok(Self)
        }
    }

    impl OtelClient for OtelDefault {
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
}

#[cfg(not(target_arch = "wasm32"))]
pub use self::host::{OtelClient, OtelDefault};
