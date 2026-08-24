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
#[cfg(feature = "otlp")]
use std::sync::OnceLock;
use std::sync::{Mutex, PoisonError};

use anyhow::Result;
#[cfg(feature = "otlp")]
use anyhow::anyhow;
#[cfg(feature = "otlp")]
use opentelemetry::trace::TracerProvider;
#[cfg(feature = "otlp")]
use opentelemetry::{KeyValue, global};
#[cfg(feature = "otlp")]
use opentelemetry_otlp::{MetricExporter, SpanExporter, WithExportConfig};
use opentelemetry_sdk::Resource;
#[cfg(feature = "otlp")]
use opentelemetry_sdk::error::{OTelSdkError, OTelSdkResult};
#[cfg(feature = "otlp")]
use opentelemetry_sdk::metrics::SdkMeterProvider;
#[cfg(feature = "otlp")]
use opentelemetry_sdk::trace::SdkTracerProvider;
#[cfg(feature = "otlp")]
use tracing_opentelemetry::MetricsLayer;
use tracing_subscriber::layer::SubscriberExt;
use tracing_subscriber::util::SubscriberInitExt;
use tracing_subscriber::{EnvFilter, Registry};

// The process's telemetry state. Like the global subscriber that references
// the providers, it lives for the rest of the process.
#[cfg(feature = "otlp")]
static INSTALLED: OnceLock<Installed> = OnceLock::new();

// Serializes first-time initialization. The `OnceLock` alone cannot: `build`
// is fallible (ruling out `get_or_init`), and without this two racers could
// both pass the empty check and race on `try_init`.
static INIT: Mutex<()> = Mutex::new(());

#[cfg(feature = "otlp")]
const UNKNOWN: &str = "unknown";

/// Log preset selecting the subscriber's filter.
///
/// Direct-command deployments peel the reserved `--debug` / `--quiet` host
/// flags from argv into one of these; every preset carries the always-on
/// noisy-dependency mutes. [`Progress`](Self::Progress) is the flagless
/// default and the only preset that defers to an ambient `RUST_LOG`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LogMode {
    /// INFO-level progress; an ambient `RUST_LOG` wins when set.
    Progress,
    /// Everything off, ignoring any ambient `RUST_LOG`.
    Quiet,
    /// INFO-level progress plus backend and HTTP debug directives, ignoring
    /// any ambient `RUST_LOG`.
    Debug,
}

/// Telemetry initializer.
pub struct Telemetry {
    /// The name of the application to for the purposes of identifying the
    /// service in telemetry data.
    #[cfg_attr(not(feature = "otlp"), allow(dead_code))]
    app_name: String,

    /// OTLP gRPC endpoint override; unset defers to OpenTelemetry endpoint
    /// resolution (`OTEL_EXPORTER_OTLP_*` env vars, then `http://localhost:4317`).
    #[cfg_attr(not(feature = "otlp"), allow(dead_code))]
    endpoint: Option<String>,

    /// Log preset for the subscriber's filter; unset defers to `RUST_LOG`.
    log_mode: Option<LogMode>,
}

impl Telemetry {
    /// Create a new telemetry resource.
    #[must_use]
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            app_name: name.into(),
            endpoint: None,
            log_mode: None,
        }
    }

    /// Set the OpenTelemetry endpoint.
    #[must_use]
    pub fn endpoint(mut self, endpoint: impl Into<String>) -> Self {
        self.endpoint = Some(endpoint.into());
        self
    }

    /// Select a [`LogMode`] preset for the subscriber's filter instead of
    /// deferring to `RUST_LOG` alone.
    #[must_use]
    pub const fn log_mode(mut self, mode: LogMode) -> Self {
        self.log_mode = Some(mode);
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

        #[cfg(feature = "otlp")]
        {
            if INSTALLED.get().is_some() {
                return Ok(());
            }

            let resource = self.resource();
            let meter_provider = self.build_metrics(resource.clone())?;
            let tracer_provider = self.build_traces(resource.clone())?;

            let filter_layer = filter(self.log_mode)?;

            // Console tracing goes to stderr: stdout belongs to the guest's
            // semantic output (command mode pipes and JSON envelopes must stay
            // clean of log lines).
            let fmt_layer = tracing_subscriber::fmt::layer().with_writer(std::io::stderr);
            let tracer = tracer_provider.tracer(self.app_name);
            let tracing_layer = tracing_opentelemetry::layer().with_tracer(tracer);
            let metrics_layer = MetricsLayer::new(meter_provider.clone());

            // Install the subscriber before publishing providers globally so a
            // failed try_init does not leave orphaned globals that later builds
            // would retry against. An already-set subscriber (an embedder's own
            // tracing setup) is tolerated: their subscriber stays, omnia's
            // exporters are skipped, and the runtime keeps running.
            if let Err(error) = Registry::default()
                .with(filter_layer)
                .with(fmt_layer)
                .with(tracing_layer)
                .with(metrics_layer)
                .try_init()
            {
                tracing::warn!(%error, "a tracing subscriber is already set; omnia telemetry skipped");
                return Ok(());
            }

            global::set_meter_provider(meter_provider.clone());
            global::set_tracer_provider(tracer_provider.clone());

            INSTALLED
                .set(Installed {
                    resource,
                    tracer: tracer_provider,
                    meter: meter_provider,
                })
                .map_err(|_installed| anyhow!("telemetry providers already installed"))
        }

        // Without the `otlp` feature there are no providers to install: the
        // subscriber (filter + fmt) is the whole initialization.
        #[cfg(not(feature = "otlp"))]
        {
            let filter_layer = filter(self.log_mode)?;
            let fmt_layer = tracing_subscriber::fmt::layer().with_writer(std::io::stderr);
            if let Err(error) = Registry::default().with(filter_layer).with(fmt_layer).try_init() {
                tracing::warn!(%error, "a tracing subscriber is already set; omnia telemetry skipped");
            }
            Ok(())
        }
    }

    #[cfg(feature = "otlp")]
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

    #[cfg(feature = "otlp")]
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

    #[cfg(feature = "otlp")]
    fn resource(&self) -> Resource {
        Resource::builder()
            .with_service_name(self.app_name.clone())
            .with_attributes(vec![
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

// The subscriber's filter for a log preset. `Quiet` and `Debug` ignore any
// ambient `RUST_LOG` (an explicit flag wins over the environment); `Progress`
// defers to `RUST_LOG` when set, else INFO. `None` keeps the historical
// env-only behavior. Every non-quiet filter carries the noisy-dependency
// mutes so bare INFO output stays readable without a hand-written suffix;
// the presets also mute the flush-failure warnings a collectorless CLI run
// would otherwise print at every exit.
fn filter(mode: Option<LogMode>) -> Result<EnvFilter> {
    let base = match mode {
        Some(LogMode::Quiet) => return Ok(EnvFilter::new("off")),
        Some(LogMode::Debug) => EnvFilter::new("info")
            .add_directive("omnia=debug".parse()?)
            .add_directive("omnia_cursor=debug".parse()?)
            .add_directive("omnia_wasi_http=debug".parse()?),
        Some(LogMode::Progress) if env::var_os(EnvFilter::DEFAULT_ENV).is_none() => {
            EnvFilter::new("info")
        }
        Some(LogMode::Progress) | None => EnvFilter::from_default_env(),
    };
    let mut base = base
        .add_directive("hyper=off".parse()?)
        .add_directive("h2=off".parse()?)
        .add_directive("tonic=off".parse()?)
        .add_directive("opentelemetry=off".parse()?)
        .add_directive("opentelemetry_sdk=off".parse()?)
        .add_directive("omnia_wasi_otel=off".parse()?);
    if mode.is_some() {
        base = base.add_directive("omnia::telemetry=off".parse()?);
    }
    Ok(base)
}

// The process's resource and provider handles.
#[cfg(feature = "otlp")]
struct Installed {
    resource: Resource,
    tracer: SdkTracerProvider,
    meter: SdkMeterProvider,
}

// Force-flush (not shut down) both providers, so telemetry keeps exporting
// afterwards and repeated flushes are safe.
#[cfg(feature = "otlp")]
fn flush_providers(tracer: &SdkTracerProvider, meter: &SdkMeterProvider) {
    settle("traces", tracer.force_flush());
    settle("metrics", meter.force_flush());
}

// Report a flush failure without panicking; a provider that is already shut
// down has nothing left to flush.
#[cfg(feature = "otlp")]
fn settle(signal: &str, result: OTelSdkResult) {
    match result {
        Ok(()) | Err(OTelSdkError::AlreadyShutdown) => {}
        Err(error) => tracing::warn!(%error, "telemetry: {signal} flush failed"),
    }
}

/// Flush batched telemetry to the exporters.
///
/// A no-op when telemetry was never initialized. This force-flushes rather
/// than shutting down, so export continues afterwards and repeated flushes
/// are safe — the runtime calls it at the end of every drive so queued spans
/// and metrics survive fast command-mode exits; embedders driving work
/// themselves should call it before the process exits.
#[cfg_attr(not(feature = "otlp"), allow(clippy::missing_const_for_fn))]
pub fn flush() {
    #[cfg(feature = "otlp")]
    if let Some(installed) = INSTALLED.get() {
        flush_providers(&installed.tracer, &installed.meter);
    }
}

/// Returns the OpenTelemetry [`Resource`] used to initialize telemetry.
///
/// `None` when telemetry has not been initialized — always the case without
/// the `otlp` feature, which builds no providers.
#[must_use]
#[cfg_attr(not(feature = "otlp"), allow(clippy::missing_const_for_fn))]
pub fn resource() -> Option<&'static Resource> {
    #[cfg(feature = "otlp")]
    {
        INSTALLED.get().map(|installed| &installed.resource)
    }
    #[cfg(not(feature = "otlp"))]
    {
        None
    }
}

// Unit tests by design: these pin the tracing/OTLP SDK contract (filter
// directives, exporter flush), not guest–host boundary behavior.
#[cfg(test)]
mod tests {
    #[cfg(feature = "otlp")]
    use std::sync::{Arc, Mutex};

    #[cfg(feature = "otlp")]
    use opentelemetry::trace::{Tracer as _, TracerProvider as _};
    #[cfg(feature = "otlp")]
    use opentelemetry_sdk::metrics::SdkMeterProvider;
    #[cfg(feature = "otlp")]
    use opentelemetry_sdk::trace::{SdkTracerProvider, SpanData, SpanExporter};

    #[cfg(feature = "otlp")]
    use super::{OTelSdkResult, flush_providers};

    // A span exporter that keeps its records, so flushing is observable
    // without a collector.
    #[cfg(feature = "otlp")]
    #[derive(Clone, Debug, Default)]
    struct Recording {
        names: Arc<Mutex<Vec<String>>>,
    }

    #[cfg(feature = "otlp")]
    impl Recording {
        fn names(&self) -> Vec<String> {
            self.names.lock().expect("recording lock").clone()
        }
    }

    #[cfg(feature = "otlp")]
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
    #[cfg(feature = "otlp")]
    fn providers(exporter: &Recording) -> (SdkTracerProvider, SdkMeterProvider) {
        (
            SdkTracerProvider::builder().with_batch_exporter(exporter.clone()).build(),
            SdkMeterProvider::builder().build(),
        )
    }

    // Filter presets are pure over their `LogMode`; the `Progress`/`None`
    // env-sensitive arms are exercised via directive membership rather than
    // mutating the process environment (other tests run in parallel).
    mod filter {
        use super::super::{LogMode, filter};

        fn directives(mode: Option<LogMode>) -> String {
            filter(mode).expect("filter directives parse").to_string()
        }

        #[test]
        fn quiet_is_off() {
            assert_eq!(directives(Some(LogMode::Quiet)), "off");
        }

        #[test]
        fn debug_filter() {
            let rendered = directives(Some(LogMode::Debug));
            for directive in [
                "info",
                "omnia=debug",
                "omnia_cursor=debug",
                "omnia_wasi_http=debug",
                "opentelemetry=off",
                "opentelemetry_sdk=off",
                "omnia_wasi_otel=off",
                "omnia::telemetry=off",
            ] {
                assert!(rendered.contains(directive), "missing `{directive}` in `{rendered}`");
            }
        }

        #[test]
        fn env_default() {
            let rendered = directives(None);
            for directive in ["hyper=off", "opentelemetry=off", "omnia_wasi_otel=off"] {
                assert!(rendered.contains(directive), "missing `{directive}` in `{rendered}`");
            }
            assert!(
                !rendered.contains("omnia::telemetry=off"),
                "the env-only path keeps flush warnings visible: `{rendered}`"
            );
        }
    }

    // The fast-exit contract: a span emitted immediately before a flush
    // reaches the exporter, and export keeps working after the flush.
    #[cfg(feature = "otlp")]
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
