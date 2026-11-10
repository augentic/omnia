mod default_impl;
mod metrics_impl;
mod resource_impl;
mod tracing_impl;
mod types_impl;

mod generated {

    pub use self::omnia::otel::types::Error;

    wasmtime::component::bindgen!({
        world: "imports",
        path: "wit",
        imports: {
            default: tracing | trappable,
        },
        with: {
            "wasi:clocks": wasmtime_wasi::p2::bindings::clocks,
        },
        trappable_error_type: {
            "omnia:otel/types.error" => Error,
        }
    });
}

use std::fmt::Debug;

use omnia_core::{FutureResult, Host, HostCtx, Provides, StoreCtx};
use opentelemetry_proto::tonic::collector::metrics::v1::ExportMetricsServiceRequest;
use opentelemetry_proto::tonic::collector::trace::v1::ExportTraceServiceRequest;
use wasmtime::component::Linker;

pub use self::default_impl::OtelDefault;
use self::generated::omnia::otel::{metrics, resource, tracing, types};

/// Host-side service for `wasi:otel`.
#[derive(Debug, HostCtx)]
#[host_ctx(client = OtelClient)]
pub struct WasiOtel;

impl<B: Provides<Self>> Host<B> for WasiOtel {
    fn add_to_linker(linker: &mut Linker<StoreCtx<B>>) -> anyhow::Result<()> {
        tracing::add_to_linker::<_, Self>(linker, StoreCtx::view::<Self>)?;
        metrics::add_to_linker::<_, Self>(linker, StoreCtx::view::<Self>)?;
        types::add_to_linker::<_, Self>(linker, StoreCtx::view::<Self>)?;
        Ok(resource::add_to_linker::<_, Self>(linker, StoreCtx::view::<Self>)?)
    }
}

/// Service contract for `omnia:otel` backends.
///
/// This is implemented by the resource-specific provider of OpenTelemetry
/// functionality.
pub trait OtelClient: Debug + Send + Sync + 'static {
    /// Export traces using gRPC.
    ///
    /// Errors are logged but not propagated to prevent telemetry failures
    /// from affecting application logic.
    fn export_traces(&self, request: ExportTraceServiceRequest) -> FutureResult<()>;

    /// Export metrics using gRPC.
    ///
    /// Errors are logged but not propagated to prevent telemetry failures
    /// from affecting application logic.
    fn export_metrics(&self, request: ExportMetricsServiceRequest) -> FutureResult<()>;
}
