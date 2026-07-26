//! # Telemetry
//!
//! Host-side OpenTelemetry initialization and OTLP exporters used to report
//! runtime telemetry out-of-the-box.
//!
//! The providers are process-global: the first [`Telemetry::build`] installs
//! them along with the global tracing subscriber, and later builds in the
//! same process are no-ops that reuse the first initialization. Batch
//! exporters queue telemetry, so call [`flush`] before a fast process exit;
//! the runtime does this at the end of every drive.

use std::env;
use std::sync::{Mutex, OnceLock, PoisonError};

use anyhow::{Result, anyhow};
use opentelemetry::trace::TracerProvider;
use opentelemetry::{KeyValue, global};
use opentelemetry_otlp::{MetricExporter, SpanExporter, WithExportConfig};
use opentelemetry_sdk::Resource;
use opentelemetry_sdk::error::{OTelSdkError, OTelSdkResult};
use opentelemetry_sdk::metrics::SdkMeterProvider;
use opentelemetry_sdk::trace::SdkTracerProvider;
use tracing_opentelemetry::MetricsLayer;
use tracing_subscriber::layer::SubscriberExt;
use tracing_subscriber::util::SubscriberInitExt;
use tracing_subscriber::{EnvFilter, Registry};

static RESOURCE: OnceLock<Resource> = OnceLock::new();

// The process's provider handles, kept for flushing. Like the global
// subscriber that references them, they live for the rest of the process.
static PROVIDERS: OnceLock<Providers> = OnceLock::new();

// Serializes first-time initialization so concurrent builds cannot race
// between the reuse check and provider installation.
static INIT: Mutex<()> = Mutex::new(());

const UNKNOWN: &str = "unknown";

/// Telemetry initializer.
pub struct Telemetry {
    /// The name of the application to for the purposes of identifying the
    /// service in telemetry data.
    app_name: String,

    /// The name of the environment, e.g. "production", "staging", "development".
    env_name: Option<String>,

    /// OTLP gRPC endpoint override; unset defers to OpenTelemetry endpoint
    /// resolution (`OTEL_EXPORTER_OTLP_*` env vars, then `http://localhost:4317`).
    endpoint: Option<String>,
}

impl Telemetry {
    /// Create a new telemetry resource.
    #[must_use]
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            app_name: name.into(),
            env_name: None,
            endpoint: None,
        }
    }

    /// Set the environment name.
    #[must_use]
    pub fn env(mut self, env_name: impl Into<String>) -> Self {
        self.env_name = Some(env_name.into());
        self
    }

    /// Set the OpenTelemetry endpoint.
    #[must_use]
    pub fn endpoint(mut self, endpoint: impl Into<String>) -> Self {
        self.endpoint = Some(endpoint.into());
        self
    }

    /// Initializes telemetry using the provided configuration.
    ///
    /// The first call in the process installs the global subscriber and
    /// providers; later calls are no-ops that reuse them (this builder's
    /// configuration is ignored), so embedders and the runtime can each
    /// initialize without coordinating.
    ///
    /// # Errors
    ///
    /// Returns an error if the telemetry system fails to initialize, such as if
    /// the OpenTelemetry exporter cannot be created or if setting the global
    /// subscriber fails.
    pub fn build(self) -> Result<()> {
        let _init = INIT.lock().unwrap_or_else(PoisonError::into_inner);
        if PROVIDERS.get().is_some() {
            return Ok(());
        }

        let resource = self.resource();
        RESOURCE.set(resource.clone()).map_err(|r| anyhow!("failed to set resource: {r:?}"))?;

        // metrics
        let meter_provider = self.build_metrics(resource.clone())?;
        global::set_meter_provider(meter_provider.clone());

        // tracing
        let tracer_provider = self.build_traces(resource)?;
        global::set_tracer_provider(tracer_provider.clone());

        let filter_layer = EnvFilter::from_default_env()
            .add_directive("hyper=off".parse()?)
            .add_directive("h2=off".parse()?)
            .add_directive("tonic=off".parse()?);

        // required for stdout
        let fmt_layer = tracing_subscriber::fmt::layer();
        let tracer = tracer_provider.tracer(self.app_name);
        let tracing_layer = tracing_opentelemetry::layer().with_tracer(tracer);
        let metrics_layer = MetricsLayer::new(meter_provider.clone());

        // set global default subscriber
        Registry::default()
            .with(filter_layer)
            .with(fmt_layer)
            .with(tracing_layer)
            .with(metrics_layer)
            .try_init()?;

        PROVIDERS
            .set(Providers {
                tracer: tracer_provider,
                meter: meter_provider,
            })
            .map_err(|_providers| anyhow!("telemetry providers already installed"))
    }

    fn build_traces(&self, resource: Resource) -> Result<SdkTracerProvider> {
        let mut exporter = SpanExporter::builder().with_tonic();
        if let Some(endpoint) = &self.endpoint {
            exporter = exporter.with_endpoint(endpoint);
        }

        Ok(SdkTracerProvider::builder()
            .with_resource(resource)
            .with_batch_exporter(exporter.build()?)
            .build())
    }

    fn build_metrics(&self, resource: Resource) -> Result<SdkMeterProvider> {
        let mut exporter = MetricExporter::builder().with_tonic();
        if let Some(endpoint) = &self.endpoint {
            exporter = exporter.with_endpoint(endpoint);
        }

        Ok(SdkMeterProvider::builder()
            .with_resource(resource)
            .with_periodic_exporter(exporter.build()?)
            .build())
    }

    fn resource(&self) -> Resource {
        Resource::builder()
            .with_service_name(self.app_name.clone())
            .with_attributes(vec![
                KeyValue::new(
                    "deployment.environment",
                    self.env_name.clone().unwrap_or_else(|| UNKNOWN.to_string()),
                ),
                KeyValue::new("service.namespace", self.app_name.clone()),
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
}

// The process's provider handles.
struct Providers {
    tracer: SdkTracerProvider,
    meter: SdkMeterProvider,
}

impl Providers {
    // Force-flush (not shut down) both providers, so telemetry keeps
    // exporting afterwards and repeated flushes are safe.
    fn flush(&self) {
        settle("traces", self.tracer.force_flush());
        settle("metrics", self.meter.force_flush());
    }
}

// Report a flush failure without panicking; a provider that is already shut
// down has nothing left to flush.
fn settle(signal: &str, result: OTelSdkResult) {
    match result {
        Ok(()) | Err(OTelSdkError::AlreadyShutdown) => {}
        Err(error) => tracing::warn!(%error, "telemetry: {signal} flush failed"),
    }
}

/// Flush batched telemetry to the exporters.
///
/// A no-op when telemetry was never initialized. Export continues afterwards,
/// so call it freely — the runtime calls it at the end of every drive so
/// queued spans and metrics survive fast command-mode exits; embedders driving
/// work themselves should call it before the process exits.
pub fn flush() {
    if let Some(providers) = PROVIDERS.get() {
        providers.flush();
    }
}

/// Returns the OpenTelemetry [`Resource`] used to initialize telemetry for a
/// service, or `None` if telemetry has not been initialized.
pub fn resource() -> Option<&'static Resource> {
    RESOURCE.get()
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Mutex};

    use opentelemetry::trace::{Tracer as _, TracerProvider as _};
    use opentelemetry_sdk::metrics::SdkMeterProvider;
    use opentelemetry_sdk::trace::{SdkTracerProvider, SpanData, SpanExporter};

    use super::{OTelSdkResult, Providers};

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
        async fn export(&self, batch: Vec<SpanData>) -> OTelSdkResult {
            self.names
                .lock()
                .expect("recording lock")
                .extend(batch.into_iter().map(|span| span.name.into_owned()));
            Ok(())
        }
    }

    // A batch-exported provider set: spans stay queued (5s schedule delay)
    // until a flush pushes them, so flush behavior is what the test sees.
    fn providers(exporter: &Recording) -> Providers {
        Providers {
            tracer: SdkTracerProvider::builder().with_batch_exporter(exporter.clone()).build(),
            meter: SdkMeterProvider::builder().build(),
        }
    }

    fn emit(providers: &Providers, name: &'static str) {
        providers.tracer.tracer("test").start(name);
    }

    // The fast-exit contract: a span emitted immediately before a flush
    // reaches the exporter, and export keeps working after the flush.
    #[test]
    fn flush_exports_queued_spans() {
        let exporter = Recording::default();
        let providers = providers(&exporter);

        emit(&providers, "first-drive");
        assert!(exporter.names().is_empty());

        providers.flush();
        assert_eq!(exporter.names(), ["first-drive"]);

        emit(&providers, "second-drive");
        providers.flush();
        assert_eq!(exporter.names(), ["first-drive", "second-drive"]);
    }
}
