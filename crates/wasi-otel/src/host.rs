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
use wasmtime::component::{HasData, Linker, ResourceTable};

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

/// A backend bundle that can yield the `wasi:otel` backend for a store.
///
/// The blanket [`WasiOtelView`] impl below turns this accessor into the
/// linker-facing view on `omnia::StoreCtx<B>`; the `runtime!` macro generates
/// the bundle-side impl via [`omnia_wasi_view!`].
pub trait HasOtel: Send {
    /// Borrow the `wasi:otel` backend context.
    fn otel_ctx(&mut self) -> &mut dyn WasiOtelCtx;
}

impl<B: HasOtel + Send + 'static> WasiOtelView for omnia::StoreCtx<B> {
    fn otel(&mut self) -> WasiOtelCtxView<'_> {
        WasiOtelCtxView {
            ctx: self.backends.otel_ctx(),
            table: &mut self.base.table,
        }
    }
}

/// Generates the bundle's [`HasOtel`] impl for a `runtime!` deployment.
#[macro_export]
macro_rules! omnia_wasi_view {
    ($bundle:ty, $field_name:ident) => {
        impl $crate::HasOtel for $bundle {
            fn otel_ctx(&mut self) -> &mut dyn $crate::WasiOtelCtx {
                &mut self.$field_name
            }
        }
    };
}
