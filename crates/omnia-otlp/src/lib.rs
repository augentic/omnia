#![doc = include_str!("../README.md")]
#![cfg(not(target_arch = "wasm32"))]

use std::env;
use std::sync::OnceLock;

use anyhow::{Result, anyhow};
use omnia_core::telemetry::{Installed, Telemetry};
use opentelemetry::trace::TracerProvider;
use opentelemetry::{KeyValue, global};
use opentelemetry_otlp::{MetricExporter, SpanExporter, WithExportConfig};
use opentelemetry_sdk::Resource;
use opentelemetry_sdk::error::{OTelSdkError, OTelSdkResult};
use opentelemetry_sdk::metrics::SdkMeterProvider;
use opentelemetry_sdk::trace::SdkTracerProvider;
use tracing_opentelemetry::MetricsLayer;

// The process's exporter state. Like the global subscriber that references
// the providers, it lives for the rest of the process.
static INSTALLED: OnceLock<Providers> = OnceLock::new();

const UNKNOWN: &str = "unknown";

/// OTLP exporter configuration: the service name plus an optional gRPC
/// endpoint.
#[derive(Clone, Debug)]
pub struct Exporters {
    name: String,

    /// OTLP gRPC endpoint override; unset defers to OpenTelemetry endpoint
    /// resolution (`OTEL_EXPORTER_OTLP_*` env vars, then `http://localhost:4317`).
    endpoint: Option<String>,
}

impl Exporters {
    /// Exporters identifying the process as service `name`.
    #[must_use]
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            endpoint: None,
        }
    }

    /// Sets the OTLP gRPC endpoint.
    #[must_use]
    pub fn endpoint(mut self, endpoint: impl Into<String>) -> Self {
        self.endpoint = Some(endpoint.into());
        self
    }

    /// Attaches the exporters to a console `Telemetry`; [`Exporting::build`]
    /// installs both.
    #[must_use]
    pub const fn attach(self, telemetry: Telemetry) -> Exporting {
        Exporting {
            exporters: self,
            telemetry,
        }
    }
}

/// A `Telemetry` with OTLP exporters attached.
pub struct Exporting {
    exporters: Exporters,
    telemetry: Telemetry,
}

impl Exporting {
    /// Installs the console subscriber with the OTLP exporters layered
    /// beneath it and publishes the providers process-wide.
    ///
    /// The first call in the process builds and publishes the providers and
    /// returns [`Installed::Now`]; later calls are no-ops that reuse them and
    /// return [`Installed::Already`].
    ///
    /// # Errors
    ///
    /// Returns an error if an exporter cannot be constructed or the
    /// subscriber fails to install.
    pub fn build(self) -> Result<Installed> {
        if INSTALLED.get().is_some() {
            return Ok(Installed::Already);
        }

        let providers = Providers::build(&self.exporters)?;
        let tracer = providers.tracer.tracer(self.exporters.name);
        let installed = self
            .telemetry
            .layer(tracing_opentelemetry::layer().with_tracer(tracer))
            .layer(MetricsLayer::new(providers.meter.clone()))
            .build()?;

        // Publish only once the subscriber that references the providers is
        // installed: a build that yielded to an already-set subscriber (an
        // embedder's own tracing setup) must not leave orphaned globals that
        // later builds would retry against.
        if installed == Installed::Now {
            providers.publish()?;
        }
        Ok(installed)
    }
}

// The process's resource and provider handles.
struct Providers {
    resource: Resource,
    tracer: SdkTracerProvider,
    meter: SdkMeterProvider,
}

impl Providers {
    fn build(exporters: &Exporters) -> Result<Self> {
        let resource = resource_for(&exporters.name);
        Ok(Self {
            tracer: build_traces(exporters.endpoint.as_deref(), resource.clone())?,
            meter: build_metrics(exporters.endpoint.as_deref(), resource.clone())?,
            resource,
        })
    }

    fn publish(self) -> Result<()> {
        global::set_meter_provider(self.meter.clone());
        global::set_tracer_provider(self.tracer.clone());
        INSTALLED.set(self).map_err(|_providers| anyhow!("telemetry providers already installed"))
    }
}

fn build_traces(endpoint: Option<&str>, resource: Resource) -> Result<SdkTracerProvider> {
    let mut exporter = SpanExporter::builder().with_tonic();
    if let Some(endpoint) = endpoint {
        exporter = exporter.with_endpoint(endpoint);
    }

    Ok(SdkTracerProvider::builder()
        .with_resource(resource)
        .with_batch_exporter(exporter.build()?)
        .build())
}

fn build_metrics(endpoint: Option<&str>, resource: Resource) -> Result<SdkMeterProvider> {
    let mut exporter = MetricExporter::builder().with_tonic();
    if let Some(endpoint) = endpoint {
        exporter = exporter.with_endpoint(endpoint);
    }

    Ok(SdkMeterProvider::builder()
        .with_resource(resource)
        .with_periodic_exporter(exporter.build()?)
        .build())
}

fn resource_for(name: &str) -> Resource {
    Resource::builder()
        .with_service_name(name.to_string())
        .with_attributes(vec![
            KeyValue::new("service.namespace", name.to_string()),
            KeyValue::new("service.version", env!("CARGO_PKG_VERSION")),
            KeyValue::new(
                "service.instance.id",
                env::var("HOSTNAME").unwrap_or_else(|_| UNKNOWN.to_string()),
            ),
            KeyValue::new("telemetry.sdk.name", "opentelemetry"),
            KeyValue::new("instrumentation.provider", "opentelemetry"),
        ])
        .build()
}

// Force-flush (not shut down) both providers, so telemetry keeps exporting
// afterwards and repeated flushes are safe.
fn flush_providers(tracer: &SdkTracerProvider, meter: &SdkMeterProvider) {
    settle("traces", tracer.force_flush());
    settle("metrics", meter.force_flush());
}

// Report a flush failure without panicking; a provider that is already shut
// down has nothing left to flush. The report is DEBUG, not WARN: a
// collectorless command-mode run fails its flush at every exit, and that is
// a deployment fact, not a warning for the console.
fn settle(signal: &str, result: OTelSdkResult) {
    match result {
        Ok(()) | Err(OTelSdkError::AlreadyShutdown) => {}
        Err(error) => tracing::debug!(%error, "telemetry: {signal} flush failed"),
    }
}

/// Flush batched telemetry to the exporters.
///
/// A no-op when the exporters were never installed. This force-flushes rather
/// than shutting down, so export continues afterwards and repeated flushes
/// are safe — the runtime calls it at the end of every drive so queued spans
/// and metrics survive fast command-mode exits; embedders driving work
/// themselves should call it before the process exits.
pub fn flush() {
    if let Some(providers) = INSTALLED.get() {
        flush_providers(&providers.tracer, &providers.meter);
    }
}

/// Returns the OpenTelemetry [`Resource`] the exporters were installed with.
///
/// `None` when the exporters have not been installed.
#[must_use]
pub fn resource() -> Option<&'static Resource> {
    INSTALLED.get().map(|providers| &providers.resource)
}

// Unit tests by design: these pin the OTLP SDK contract (exporter flush),
// not guest–host boundary behavior.
#[cfg(test)]
mod tests {
    use std::sync::{Arc, Mutex};

    use opentelemetry::trace::{Tracer as _, TracerProvider as _};
    use opentelemetry_sdk::metrics::SdkMeterProvider;
    use opentelemetry_sdk::trace::{SdkTracerProvider, SpanData, SpanExporter};

    use super::{OTelSdkResult, flush_providers};

    // A span exporter that keeps its records, so flushing is observable
    // without a collector.
    #[derive(Clone, Debug, Default)]
    struct Recording {
        names: Arc<Mutex<Vec<String>>>,
    }

    impl Recording {
        fn names(&self) -> Vec<String> {
            self.names.lock().expect("recording lock").clone()
        }
    }

    impl SpanExporter for Recording {
        fn export(&self, batch: Vec<SpanData>) -> impl std::future::Future<Output = OTelSdkResult> {
            self.names
                .lock()
                .expect("recording lock")
                .extend(batch.into_iter().map(|span| span.name.into_owned()));
            std::future::ready(Ok(()))
        }
    }

    // A batch-exported provider set: spans stay queued (5s schedule delay)
    // until a flush pushes them, so flush behavior is what the test sees.
    fn providers(exporter: &Recording) -> (SdkTracerProvider, SdkMeterProvider) {
        (
            SdkTracerProvider::builder().with_batch_exporter(exporter.clone()).build(),
            SdkMeterProvider::builder().build(),
        )
    }

    // The fast-exit contract: a span emitted immediately before a flush
    // reaches the exporter, and export keeps working after the flush.
    #[test]
    fn flush_exports() {
        let exporter = Recording::default();
        let (tracer, meter) = providers(&exporter);

        tracer.tracer("test").start("first-drive");
        assert!(exporter.names().is_empty());

        flush_providers(&tracer, &meter);
        assert_eq!(exporter.names(), ["first-drive"]);

        tracer.tracer("test").start("second-drive");
        flush_providers(&tracer, &meter);
        assert_eq!(exporter.names(), ["first-drive", "second-drive"]);
    }
}
