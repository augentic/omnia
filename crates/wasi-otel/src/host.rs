mod default_impl;
mod metrics_impl;
mod resource_impl;
mod tracing_impl;
mod types_impl;

mod generated {
    #![allow(clippy::trait_duplication_in_bounds)]
    pub use self::wasi::otel::types::Error;

    wasmtime::component::bindgen!({
        world: "otel",
        path: "wit",
        imports: {
            "wasi:otel/resource.resource": tracing | trappable,
            default: store | tracing | trappable,
        },
        trappable_error_type: {
            "wasi:otel/types.error" => Error,
        }
    });
}

use std::fmt::Debug;

use opentelemetry_proto::tonic::collector::metrics::v1::ExportMetricsServiceRequest;
use opentelemetry_proto::tonic::collector::trace::v1::ExportTraceServiceRequest;
use wasmtime::component::{HasData, Linker, ResourceTable};
use yetti::{FutureResult, Host, Server, State};

pub use self::default_impl::OtelDefault;
use self::generated::wasi::otel::{metrics, resource, tracing, types};

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
        resource::add_to_linker::<_, Self>(linker, T::otel)
    }
}

impl<S: State> Server<S> for WasiOtel {}

/// A trait which provides internal WASI OpenTelemetry state.
///
/// This is implemented by the `T` in `Linker<T>` — a single type shared across
/// all WASI components for the runtime build.
pub trait WasiOtelView: Send {
    /// Return a [`WasiOtelCtxView`] from mutable reference to self.
    fn otel(&mut self) -> WasiOtelCtxView<'_>;
}

/// View into [`WasiOtelCtx`] implementation and [`ResourceTable`].
pub struct WasiOtelCtxView<'a> {
    /// Mutable reference to the WASI OpenTelemetry context.
    pub ctx: &'a mut dyn WasiOtelCtx,

    /// Mutable reference to table used to manage resources.
    pub table: &'a mut ResourceTable,
}

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

#[macro_export]
macro_rules! wasi_view {
    ($store_ctx:ty, $field_name:ident) => {
        impl yetti_wasi_otel::WasiOtelView for $store_ctx {
            fn otel(&mut self) -> yetti_wasi_otel::WasiOtelCtxView<'_> {
                yetti_wasi_otel::WasiOtelCtxView {
                    ctx: &mut self.$field_name,
                    table: &mut self.table,
                }
            }
        }
    };
}
