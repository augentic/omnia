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

#[cfg(feature = "otlp")]
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
use tracing_subscriber::filter::LevelFilter;
use tracing_subscriber::layer::{Layer, SubscriberExt};
use tracing_subscriber::util::SubscriberInitExt;
use tracing_subscriber::{EnvFilter, Registry};

// The process's telemetry state. Like the global subscriber that references
// the providers, it lives for the rest of the process.
#[cfg(feature = "otlp")]
static INSTALLED: OnceLock<Providers> = OnceLock::new();

// Serializes first-time initialization. The `OnceLock` alone cannot: `build`
// is fallible (ruling out `get_or_init`), and without this two racers could
// both pass the empty check and race on `try_init`.
static INIT: Mutex<()> = Mutex::new(());

#[cfg(feature = "otlp")]
const UNKNOWN: &str = "unknown";

/// Outcome of [`Telemetry::build`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Installed {
    /// This call installed omnia's subscriber.
    Now,
    /// A subscriber was already set (omnia's own earlier build or an
    /// embedder's); nothing was installed.
    Already,
}

/// Telemetry initializer.
pub struct Telemetry {
    /// The name of the application to for the purposes of identifying the
    /// service in telemetry data.
    #[cfg_attr(not(feature = "otlp"), allow(dead_code))]
    app_name: String,

    /// Layers installed beneath the console layers (exporters attach here).
    layers: Vec<Box<dyn Layer<Registry> + Send + Sync>>,

    /// OTLP gRPC endpoint override; unset defers to OpenTelemetry endpoint
    /// resolution (`OTEL_EXPORTER_OTLP_*` env vars, then `http://localhost:4317`).
    #[cfg_attr(not(feature = "otlp"), allow(dead_code))]
    endpoint: Option<String>,

    /// Explicit filter directives for the console subscriber; unset defers
    /// to `RUST_LOG`.
    filter: Option<String>,

    /// The level the console subscriber falls back to when `RUST_LOG` is
    /// unset and no explicit directives are given.
    fallback: LevelFilter,
}

impl Telemetry {
    /// Create a new telemetry resource.
    #[must_use]
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            app_name: name.into(),
            layers: Vec::new(),
            endpoint: None,
            filter: None,
            fallback: LevelFilter::WARN,
        }
    }

    /// Set the OpenTelemetry endpoint.
    #[must_use]
    pub fn endpoint(mut self, endpoint: impl Into<String>) -> Self {
        self.endpoint = Some(endpoint.into());
        self
    }

    /// Adds a subscriber layer beneath the console layers (an exporter
    /// attaches here).
    #[must_use]
    pub fn layer(mut self, layer: impl Layer<Registry> + Send + Sync + 'static) -> Self {
        self.layers.push(Box::new(layer));
        self
    }

    /// Filters the console subscriber by `directives` instead of the
    /// environment.
    ///
    /// `directives` is a `RUST_LOG` string (`info`, `omnia_core=debug`).
    /// The always-on noisy-dependency mutes still apply.
    #[must_use]
    pub fn filter(mut self, directives: impl Into<String>) -> Self {
        self.filter = Some(directives.into());
        self
    }

    /// Sets the level the console subscriber falls back to when the
    /// environment sets no `RUST_LOG`.
    ///
    /// `WARN` when not called. Explicit [`filter`](Self::filter) directives
    /// take precedence over both.
    #[must_use]
    pub const fn fallback(mut self, level: LevelFilter) -> Self {
        self.fallback = level;
        self
    }

    /// Initializes telemetry using the provided configuration.
    ///
    /// The first call in the process installs the global subscriber and
    /// providers and returns [`Installed::Now`]; later calls are no-ops that
    /// reuse them (this builder's configuration is ignored) and return
    /// [`Installed::Already`], so embedders and the runtime can each
    /// initialize without coordinating.
    ///
    /// # Errors
    ///
    /// Returns an error if the telemetry system fails to initialize, such as if
    /// the OpenTelemetry exporter cannot be created or if setting the global
    /// subscriber fails.
    pub fn build(self) -> Result<Installed> {
        let _init = INIT.lock().unwrap_or_else(PoisonError::into_inner);

        #[cfg(feature = "otlp")]
        {
            if INSTALLED.get().is_some() {
                return Ok(Installed::Already);
            }

            let resource = self.resource();
            let meter_provider = self.build_metrics(resource.clone())?;
            let tracer_provider = self.build_traces(resource.clone())?;

            let filter_layer =
                filter(self.filter.as_deref(), self.fallback, rust_log().as_deref())?;
            let layers = attached(self.layers);

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
                .with(layers)
                .with(filter_layer)
                .with(fmt_layer)
                .with(tracing_layer)
                .with(metrics_layer)
                .try_init()
            {
                tracing::warn!(%error, "a tracing subscriber is already set; omnia telemetry skipped");
                return Ok(Installed::Already);
            }

            global::set_meter_provider(meter_provider.clone());
            global::set_tracer_provider(tracer_provider.clone());

            INSTALLED
                .set(Providers {
                    resource,
                    tracer: tracer_provider,
                    meter: meter_provider,
                })
                .map_err(|_providers| anyhow!("telemetry providers already installed"))?;
            Ok(Installed::Now)
        }

        // Without the `otlp` feature there are no providers to install: the
        // subscriber (filter + fmt) is the whole initialization.
        #[cfg(not(feature = "otlp"))]
        {
            let filter_layer =
                filter(self.filter.as_deref(), self.fallback, rust_log().as_deref())?;
            let fmt_layer = tracing_subscriber::fmt::layer().with_writer(std::io::stderr);
            let layers = attached(self.layers);
            if let Err(error) =
                Registry::default().with(layers).with(filter_layer).with(fmt_layer).try_init()
            {
                tracing::warn!(%error, "a tracing subscriber is already set; omnia telemetry skipped");
                return Ok(Installed::Already);
            }
            Ok(Installed::Now)
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

// The attached layers as one `Layer`, or `None` when there are none. An empty
// `Vec` is not a no-op layer: it registers every callsite as `Interest::never`
// and hints `LevelFilter::OFF`, which as the innermost layer silences the whole
// stack. `Option::None` is tracing-subscriber's no-op, and `Layered` overrides
// its hint.
fn attached(
    layers: Vec<Box<dyn Layer<Registry> + Send + Sync>>,
) -> Option<Vec<Box<dyn Layer<Registry> + Send + Sync>>> {
    (!layers.is_empty()).then_some(layers)
}

// The process's `RUST_LOG`, read once per build so `filter` stays pure over
// its inputs.
fn rust_log() -> Option<String> {
    std::env::var("RUST_LOG").ok()
}

// The subscriber's filter: explicit `directives` when given, else the
// `rust_log` directives with `fallback` as the level an unset variable falls
// back to, so host warnings (a mount that failed to preopen) reach the
// console without a hand-written filter. Every filter carries the
// noisy-dependency mutes so the output stays readable without a hand-written
// suffix.
fn filter(
    directives: Option<&str>, fallback: LevelFilter, rust_log: Option<&str>,
) -> Result<EnvFilter> {
    let base = match directives {
        Some(directives) => EnvFilter::builder().parse(directives)?,
        None => EnvFilter::builder()
            .with_default_directive(fallback.into())
            .parse_lossy(rust_log.unwrap_or_default()),
    };
    Ok(base
        .add_directive("hyper=off".parse()?)
        .add_directive("h2=off".parse()?)
        .add_directive("tonic=off".parse()?)
        .add_directive("opentelemetry=off".parse()?)
        .add_directive("opentelemetry_sdk=off".parse()?)
        .add_directive("omnia_wasi_otel=off".parse()?))
}

// The process's resource and provider handles.
#[cfg(feature = "otlp")]
struct Providers {
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
// down has nothing left to flush. The report is DEBUG, not WARN: a
// collectorless command-mode run fails its flush at every exit, and that is
// a deployment fact, not a warning for the console.
#[cfg(feature = "otlp")]
fn settle(signal: &str, result: OTelSdkResult) {
    match result {
        Ok(()) | Err(OTelSdkError::AlreadyShutdown) => {}
        Err(error) => tracing::debug!(%error, "telemetry: {signal} flush failed"),
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
    if let Some(providers) = INSTALLED.get() {
        flush_providers(&providers.tracer, &providers.meter);
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
        INSTALLED.get().map(|providers| &providers.resource)
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

    // Pure over its inputs: the process `RUST_LOG` is handed in, never read
    // (other tests run in parallel).
    mod filter {
        use super::super::{LevelFilter, filter};

        const MUTES: [&str; 6] = [
            "hyper=off",
            "h2=off",
            "tonic=off",
            "opentelemetry=off",
            "opentelemetry_sdk=off",
            "omnia_wasi_otel=off",
        ];

        fn rendered(
            directives: Option<&str>, fallback: LevelFilter, rust_log: Option<&str>,
        ) -> String {
            filter(directives, fallback, rust_log).expect("filter directives parse").to_string()
        }

        fn has(rendered: &str, directive: &str) -> bool {
            rendered.split(',').any(|candidate| candidate == directive)
        }

        #[test]
        fn explicit_directives() {
            let rendered =
                rendered(Some("info,omnia_core=debug"), LevelFilter::WARN, Some("trace"));
            for directive in ["info", "omnia_core=debug"].into_iter().chain(MUTES) {
                assert!(has(&rendered, directive), "missing `{directive}` in `{rendered}`");
            }
            assert!(!has(&rendered, "trace"), "explicit directives replace `RUST_LOG`: {rendered}");
        }

        #[test]
        fn explicit_off() {
            let rendered = rendered(Some("off"), LevelFilter::WARN, None);
            assert!(has(&rendered, "off"), "{rendered}");
        }

        #[test]
        fn fallback_level() {
            let rendered = rendered(None, LevelFilter::INFO, None);
            assert!(has(&rendered, "info"), "an unset `RUST_LOG` falls back: {rendered}");
            for directive in MUTES {
                assert!(has(&rendered, directive), "missing `{directive}` in `{rendered}`");
            }
        }

        #[test]
        fn rust_log_over_fallback() {
            let rendered = rendered(None, LevelFilter::INFO, Some("omnia_core=debug"));
            assert!(has(&rendered, "omnia_core=debug"), "{rendered}");
            assert!(!has(&rendered, "info"), "a set `RUST_LOG` displaces the fallback: {rendered}");
        }

        #[test]
        fn unparsable_directives() {
            assert!(
                filter(Some("omnia_core=loud"), LevelFilter::WARN, None).is_err(),
                "an unknown level must not parse"
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
