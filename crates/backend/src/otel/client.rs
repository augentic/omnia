//! Service contract for `omnia:otel` backends.

use std::fmt::Debug;

use opentelemetry_proto::tonic::collector::metrics::v1::ExportMetricsServiceRequest;
use opentelemetry_proto::tonic::collector::trace::v1::ExportTraceServiceRequest;

use crate::FutureResult;

/// Service contract for `omnia:otel` backends.
pub trait OtelClient: Debug + Send + Sync + 'static {
    /// Export traces using gRPC.
    fn export_traces(&self, request: ExportTraceServiceRequest) -> FutureResult<()>;

    /// Export metrics using gRPC.
    fn export_metrics(&self, request: ExportMetricsServiceRequest) -> FutureResult<()>;
}
