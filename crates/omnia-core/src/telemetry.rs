//! # Telemetry
//!
//! The host's observability stack: the `tracing` subscriber (console
//! `EnvFilter` + `fmt` to stderr) with the OTLP span and metric exporters
//! layered beneath it, and the process-wide OpenTelemetry providers they
//! publish.
//!
//! Telemetry is process-global: the first [`Telemetry::build`] installs the
//! subscriber and the providers, and later builds in the same process are
//! no-ops that reuse the first initialization. Batch exporters queue
//! telemetry, so call [`flush`] before a fast process exit; the runtime does
//! this at the end of every drive.

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
use tracing_subscriber::filter::LevelFilter;
use tracing_subscriber::layer::SubscriberExt;
use tracing_subscriber::util::SubscriberInitExt;
use tracing_subscriber::{EnvFilter, Registry};

// Whether the process's global subscriber is settled: omnia's installed, or
// an embedder's found already set (a global subscriber is never replaced, so
// retrying `try_init` could only re-warn). Held for the whole of `build`, so
// two racers cannot both pass the check and race on `try_init`.
static SETTLED: Mutex<bool> = Mutex::new(false);

// The process's provider state. Like the global subscriber that references
// the providers, it lives for the rest of the process.
static PROVIDERS: OnceLock<Providers> = OnceLock::new();

/// Builder for the host's telemetry: the `tracing` subscriber with OTLP
/// exporters beneath it.
pub struct Telemetry {
    /// The service name identifying the process in telemetry data.
    name: String,

    /// OTLP gRPC endpoint override; unset defers to OpenTelemetry endpoint
    /// resolution (`OTEL_EXPORTER_OTLP_*` env vars, then `http://localhost:4317`).
    endpoint: Option<String>,

    /// Explicit filter directives for the console; unset defers to
    /// `RUST_LOG`.
    filter: Option<String>,

    /// The level the console falls back to when `RUST_LOG` is unset and no
    /// explicit directives are given.
    fallback: LevelFilter,
}

impl Telemetry {
    /// Create a builder identifying the process as service `name`, at the
    /// `WARN` console fallback.
    #[must_use]
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            endpoint: None,
            filter: None,
            fallback: LevelFilter::WARN,
        }
    }

    /// Sets the OTLP gRPC endpoint.
    #[must_use]
    pub fn endpoint(mut self, endpoint: impl Into<String>) -> Self {
        self.endpoint = Some(endpoint.into());
        self
    }

    /// Filters the console by `directives` instead of the environment.
    ///
    /// `directives` is a `RUST_LOG` string (`info`, `omnia_core=debug`).
    /// The always-on noisy-dependency mutes still apply.
    #[must_use]
    pub fn filter(mut self, directives: impl Into<String>) -> Self {
        self.filter = Some(directives.into());
        self
    }

    /// Sets the level the console falls back to when the environment sets no
    /// `RUST_LOG`.
    ///
    /// `WARN` when not called. Explicit [`filter`](Self::filter) directives
    /// take precedence over both.
    #[must_use]
    pub const fn fallback(mut self, level: LevelFilter) -> Self {
        self.fallback = level;
        self
    }

    /// Installs the subscriber as the process's global `tracing` subscriber
    /// and publishes the OpenTelemetry providers process-wide.
    ///
    /// # Errors
    ///
    /// Returns an error if the filter directives do not parse or an exporter
    /// cannot be constructed.
    pub fn build(self) -> Result<()> {
        let mut settled = SETTLED.lock().unwrap_or_else(PoisonError::into_inner);
        if *settled {
            return Ok(());
        }

        let filter_layer = filter(self.filter.as_deref(), self.fallback, rust_log().as_deref())?;
        // Console tracing goes to stderr: stdout belongs to the guest's output.
        let fmt_layer = tracing_subscriber::fmt::layer().with_writer(std::io::stderr);

        let providers = Providers::build(&self.name, self.endpoint.as_deref())?;
        let tracer = providers.tracer.tracer(self.name);

        // Publish the providers only once the subscriber that references
        // them is installed.
        match Registry::default()
            .with(filter_layer)
            .with(fmt_layer)
            .with(tracing_opentelemetry::layer().with_tracer(tracer))
            .with(MetricsLayer::new(providers.meter.clone()))
            .try_init()
        {
            Ok(()) => providers.publish()?,
            Err(error) => {
                tracing::warn!(%error, "a tracing subscriber is already set; omnia's skipped");
            }
        }
        *settled = true;
        drop(settled);
        Ok(())
    }
}

// The process's `RUST_LOG`, read once per build so `filter`.
fn rust_log() -> Option<String> {
    env::var("RUST_LOG").ok()
}

// The subscriber's filter: explicit `directives` when given, else the
// `rust_log` directives with `fallback` as the level an unset variable falls
// back to.
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
struct Providers {
    resource: Resource,
    tracer: SdkTracerProvider,
    meter: SdkMeterProvider,
}

impl Providers {
    fn build(name: &str, endpoint: Option<&str>) -> Result<Self> {
        let resource = resource_for(name);
        Ok(Self {
            tracer: build_traces(endpoint, resource.clone())?,
            meter: build_metrics(endpoint, resource.clone())?,
            resource,
        })
    }

    fn publish(self) -> Result<()> {
        global::set_meter_provider(self.meter.clone());
        global::set_tracer_provider(self.tracer.clone());
        PROVIDERS.set(self).map_err(|_providers| anyhow!("telemetry providers already installed"))
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
                env::var("HOSTNAME").unwrap_or_else(|_| "unknown".to_string()),
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
// down has nothing left to flush.
fn settle(signal: &str, result: OTelSdkResult) {
    match result {
        Ok(()) | Err(OTelSdkError::AlreadyShutdown) => {}
        Err(error) => tracing::debug!(%error, "telemetry: {signal} flush failed"),
    }
}

/// Flush batched telemetry to the exporters.
///
/// A no-op when telemetry was never installed.
pub fn flush() {
    if let Some(providers) = PROVIDERS.get() {
        flush_providers(&providers.tracer, &providers.meter);
    }
}

/// Returns the OpenTelemetry [`Resource`] telemetry was installed with.
///
/// `None` when telemetry has not been installed.
#[must_use]
pub fn resource() -> Option<&'static Resource> {
    PROVIDERS.get().map(|providers| &providers.resource)
}

// Unit tests by design: these pin the tracing/OTLP SDK contract (filter
// directives, exporter flush), not guest–host boundary behavior.
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
