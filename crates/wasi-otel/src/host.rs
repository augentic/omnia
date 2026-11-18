//! # WASI OpenTelemetry Service

mod default_impl;
mod logs_impl;
mod metrics_impl;
mod traces_impl;
mod types_impl;

mod generated {
    pub use self::omnia::otel::types::Error;

    wasmtime::component::bindgen!({
        world: "imports",
        path: "wit",
        imports: {
            default: store | tracing | trappable,
        },
        trappable_error_type: {
            "omnia:otel/types.error" => Error,
        },
    });
}

use std::fmt::Debug;

use omnia_core::{FutureResult, Host, HostCtx, Provides, StoreCtx};
use opentelemetry_proto::tonic::collector::metrics::v1::ExportMetricsServiceRequest;
use opentelemetry_proto::tonic::collector::trace::v1::ExportTraceServiceRequest;
use wasmtime::component::Linker;

pub use self::default_impl::OtelDefault;
pub use self::generated::Error;
use self::generated::omnia::otel::{logs, metrics, traces, types};

type Result<T> = std::result::Result<T, Error>;

/// Host-side service for `omnia:otel`.
#[derive(Debug, HostCtx)]
#[host_ctx(client = OtelClient)]
pub struct WasiOtel;

impl<B: Provides<Self>> Host<B> for WasiOtel {
    fn add_to_linker(linker: &mut Linker<StoreCtx<B>>) -> anyhow::Result<()> {
        types::add_to_linker::<_, Self>(linker, StoreCtx::view::<Self>)?;
        traces::add_to_linker::<_, Self>(linker, StoreCtx::view::<Self>)?;
        metrics::add_to_linker::<_, Self>(linker, StoreCtx::view::<Self>)?;
        Ok(logs::add_to_linker::<_, Self>(linker, StoreCtx::view::<Self>)?)
    }
}

/// Service contract for `omnia:otel` backends.
///
/// This is implemented by the resource-specific provider of OpenTelemetry
/// functionality. Each request carries the deployment's resource and the
/// guest's telemetry, already checked.
///
/// A backend picks a failure's kind by returning an [`Error`] inside its
/// [`anyhow::Error`], and any other failure is [`Error::Other`].
pub trait OtelClient: Debug + Send + Sync + 'static {
    /// Export spans, as one OTLP trace request.
    fn export_traces(&self, request: ExportTraceServiceRequest) -> FutureResult<()>;

    /// Export metric points, as one OTLP metrics request.
    fn export_metrics(&self, request: ExportMetricsServiceRequest) -> FutureResult<()>;
}

// a backend picks the kind by returning an `Error`; any other failure is
// `other`, with its full context chain
impl From<anyhow::Error> for Error {
    fn from(err: anyhow::Error) -> Self {
        err.downcast::<Self>().unwrap_or_else(|err| Self::Other(format!("{err:#}")))
    }
}
