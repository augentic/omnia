//! The default `omnia:otel` backend, which sends guest telemetry to the
//! host's own collector.

use std::future::{Future, ready};

use anyhow::Result;
use futures::FutureExt;
use omnia_core::telemetry::{self, Forwarded};
use omnia_core::{Connect, FutureResult};
use opentelemetry_proto::tonic::collector::metrics::v1::ExportMetricsServiceRequest;
use opentelemetry_proto::tonic::collector::trace::v1::ExportTraceServiceRequest;

use crate::{Error, OtelClient};

/// The default `omnia:otel` backend, which sends guest telemetry to the
/// host's collector.
///
/// A guest's spans and metrics go where the host's own do: the endpoint,
/// headers, and timeout the `OTEL_EXPORTER_OTLP_*` variables configure for
/// the signal, over the same channel. Each export is queued and sent in the
/// background, so a slow or absent collector never holds a guest.
///
/// - With no endpoint for the signal, the telemetry is dropped and the export
///   succeeds.
/// - A full queue fails the export with [`Error::Unavailable`].
/// - An export over 4 `MiB` fails with [`Error::Invalid`].
#[derive(Debug, Clone)]
pub struct OtelDefault;

impl Connect for OtelDefault {
    type ConnectOptions = omnia_core::NoOptions;

    fn connect_with(_options: Self::ConnectOptions) -> impl Future<Output = Result<Self>> {
        ready(Ok(Self))
    }
}

impl OtelClient for OtelDefault {
    fn export_traces(&self, request: ExportTraceServiceRequest) -> FutureResult<()> {
        ready(answer(telemetry::forward_traces(request))).boxed()
    }

    fn export_metrics(&self, request: ExportMetricsServiceRequest) -> FutureResult<()> {
        ready(answer(telemetry::forward_metrics(request))).boxed()
    }
}

// a run without a collector chose to drop telemetry, which is no failure of the guest's
fn answer(forwarded: Forwarded) -> Result<()> {
    match forwarded {
        Forwarded::Queued | Forwarded::NoCollector => Ok(()),
        Forwarded::Full => {
            Err(Error::Unavailable("the host's telemetry queue is full".to_owned()).into())
        }
        Forwarded::TooLarge => {
            Err(Error::Invalid("the telemetry is over the 4 MiB one export carries".to_owned())
                .into())
        }
    }
}
