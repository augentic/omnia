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
            "omnia:otel/resource.resource": tracing | trappable,
            default: store | tracing | trappable,
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

use omnia::{FutureResult, Host, Server};
use opentelemetry_proto::tonic::collector::metrics::v1::ExportMetricsServiceRequest;
use opentelemetry_proto::tonic::collector::trace::v1::ExportTraceServiceRequest;
use wasmtime::component::{HasData, Linker};

pub use self::default_impl::OtelDefault;
use self::generated::omnia::otel::{metrics, resource, tracing, types};

/// Host-side service for `wasi:otel`.
#[derive(Debug)]
pub struct WasiOtel;

impl HasData for WasiOtel {
    type Data<'a> = WasiOtelCtxView<'a>;
}

impl<T> Host<T> for WasiOtel
where
    T: WasiOtelView + 'static,
{
    fn add_to_linker(linker: &mut Linker<T>) -> anyhow::Result<()> {
        tracing::add_to_linker::<_, Self>(linker, T::otel)?;
        metrics::add_to_linker::<_, Self>(linker, T::otel)?;
        types::add_to_linker::<_, Self>(linker, T::otel)?;
        Ok(resource::add_to_linker::<_, Self>(linker, T::otel)?)
    }
}

impl<B> Server<B> for WasiOtel {}

/// A trait which provides internal WASI OpenTelemetry context.
///
/// This is implemented by the resource-specific provider of OpenTelemetry
/// functionality.
pub trait WasiOtelCtx: Debug + Send + Sync + 'static {
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

omnia::wasi_view!(Otel);
