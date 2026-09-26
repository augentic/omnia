//! # Telemetry
//!
//! The host's observability stack: the `tracing` subscriber (console
//! `EnvFilter` + `fmt` to stderr) with the OTLP span and metric exporters
//! layered beneath it, and the process-wide OpenTelemetry providers they
//! publish. The console filter is the run's [`directives`]; a signal's
//! exporter attaches only when an OTLP endpoint is configured for it, so a
//! run with no collector exports nothing rather than retrying against one.
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
use opentelemetry_otlp::{
    MetricExporter, OTEL_EXPORTER_OTLP_ENDPOINT, OTEL_EXPORTER_OTLP_METRICS_ENDPOINT,
    OTEL_EXPORTER_OTLP_TRACES_ENDPOINT, SpanExporter, WithExportConfig,
};
use opentelemetry_sdk::Resource;
use opentelemetry_sdk::error::{OTelSdkError, OTelSdkResult};
use opentelemetry_sdk::metrics::SdkMeterProvider;
use opentelemetry_sdk::trace::SdkTracerProvider;
use tracing_opentelemetry::MetricsLayer;
use tracing_subscriber::filter::{Directive, LevelFilter};
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
    /// resolution (`OTEL_EXPORTER_OTLP_*` env vars), and a signal with no
    /// endpoint from either has no exporter.
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

    /// Sets the OTLP gRPC endpoint both signals export to.
    #[must_use]
    pub fn endpoint(mut self, endpoint: impl Into<String>) -> Self {
        self.endpoint = Some(endpoint.into());
        self
    }

    /// Filters the console by `directives` instead of the environment.
    ///
    /// `directives` is a `RUST_LOG` string (`info`, `omnia_core=debug`) —
    /// typically the run's [`directives`], composed from its verbosity flag
    /// and the process `RUST_LOG`. The always-on noisy-dependency mutes still
    /// apply.
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

        let console =
            self.filter.unwrap_or_else(|| directives(None, self.fallback, rust_log().as_deref()));
        let filter_layer = filter(&console)?;
        // Console tracing goes to stderr: stdout belongs to the guest's output.
        let fmt_layer = tracing_subscriber::fmt::layer().with_writer(std::io::stderr);

        let exports = exports(self.endpoint.as_deref(), |name| {
            env::var(name).ok().filter(|value| !value.is_empty())
        });
        let providers = Providers::build(&self.name, self.endpoint.as_deref(), exports)?;
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
            Ok(()) => {
                providers.publish()?;
                if exports != Exports::ALL {
                    tracing::debug!(
                        traces = exports.traces,
                        metrics = exports.metrics,
                        "no OTLP endpoint (`OTEL_GRPC_URL`, `OTEL_EXPORTER_OTLP_*`); an \
                         unexported signal is dropped"
                    );
                }
            }
            Err(error) => {
                tracing::warn!(%error, "a tracing subscriber is already set; omnia's skipped");
            }
        }
        *settled = true;
        drop(settled);
        Ok(())
    }
}

// Which signals have an exporter. OpenTelemetry's endpoint resolution falls
// back to `localhost:4317`, where without a collector every export is a
// connect retry and the exit flush waits on them; so a signal exports only
// when an endpoint is configured for it — the builder's (`OTEL_GRPC_URL`),
// the shared `OTEL_EXPORTER_OTLP_ENDPOINT`, or the signal's own variable.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Exports {
    traces: bool,
    metrics: bool,
}

impl Exports {
    const ALL: Self = Self {
        traces: true,
        metrics: true,
    };
}

fn exports(endpoint: Option<&str>, env: impl Fn(&str) -> Option<String>) -> Exports {
    let shared = endpoint.is_some() || env(OTEL_EXPORTER_OTLP_ENDPOINT).is_some();
    Exports {
        traces: shared || env(OTEL_EXPORTER_OTLP_TRACES_ENDPOINT).is_some(),
        metrics: shared || env(OTEL_EXPORTER_OTLP_METRICS_ENDPOINT).is_some(),
    }
}

/// The run's tracing directives: its global level — `level` when one is
/// selected, else the bare level `rust_log` carries, else `fallback` when it
/// carries no directive at all — followed by every targeted directive
/// (`tower=off`, `omnia_core=trace`, `[span]=debug`) in `rust_log`.
///
/// A selected level displaces only `rust_log`'s bare level, so a flag steps
/// the run's level without discarding the operator's refinements. A token
/// that is neither is reported on stderr and dropped, as `EnvFilter` drops
/// it on a bare run; the result always parses.
#[must_use]
pub fn directives(
    level: Option<LevelFilter>, fallback: LevelFilter, rust_log: Option<&str>,
) -> String {
    let mut global = None;
    let mut targeted = Vec::new();
    for token in rust_log.unwrap_or_default().split(',').map(str::trim) {
        // `EnvFilter` reads a bare token as the global level exactly when it
        // parses as one; an empty token is skipped there too (and would parse
        // as `error` here).
        if token.is_empty() {
            continue;
        }
        if let Ok(level) = token.parse::<LevelFilter>() {
            global = Some(level);
        } else if let Err(error) = token.parse::<Directive>() {
            eprintln!("ignoring `RUST_LOG` directive `{token}`: {error}");
        } else {
            targeted.push(token);
        }
    }
    // The fallback fills an empty `RUST_LOG`, never one that names a target:
    // `EnvFilter`'s default directive applies only when nothing parsed, so a
    // bare `RUST_LOG=omnia_core=trace` run stays as narrow as it always was.
    let global = level.or(global).or_else(|| targeted.is_empty().then_some(fallback));
    global
        .map(|level| level.to_string())
        .into_iter()
        .chain(targeted.into_iter().map(str::to_owned))
        .collect::<Vec<_>>()
        .join(",")
}

// The process's `RUST_LOG`, read once per build so `directives` stays pure
// over its inputs.
fn rust_log() -> Option<String> {
    env::var("RUST_LOG").ok()
}

// The subscriber's filter: `directives` with the noisy-dependency mutes
// appended, so the console stays readable without a hand-written suffix.
// `tower` is muted whole: it narrates every buffered request, and it arrives
// through `reqwest` (registry fetches) as well as `tonic` (OTLP).
fn filter(directives: &str) -> Result<EnvFilter> {
    Ok(EnvFilter::builder()
        .parse(directives)?
        .add_directive("hyper=off".parse()?)
        .add_directive("h2=off".parse()?)
        .add_directive("tower=off".parse()?)
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
    // The providers are built and published whatever exports: the resource
    // and the tracer are what guest telemetry grafts onto, and a provider
    // with no processor or reader drops what it is handed.
    fn build(name: &str, endpoint: Option<&str>, exports: Exports) -> Result<Self> {
        let resource = resource_for(name);
        Ok(Self {
            tracer: build_traces(endpoint, resource.clone(), exports.traces)?,
            meter: build_metrics(endpoint, resource.clone(), exports.metrics)?,
            resource,
        })
    }

    fn publish(self) -> Result<()> {
        global::set_meter_provider(self.meter.clone());
        global::set_tracer_provider(self.tracer.clone());
        PROVIDERS.set(self).map_err(|_providers| anyhow!("telemetry providers already installed"))
    }
}

fn build_traces(
    endpoint: Option<&str>, resource: Resource, export: bool,
) -> Result<SdkTracerProvider> {
    let mut provider = SdkTracerProvider::builder().with_resource(resource);
    if export {
        let mut exporter = SpanExporter::builder().with_tonic();
        if let Some(endpoint) = endpoint {
            exporter = exporter.with_endpoint(endpoint);
        }
        provider = provider.with_batch_exporter(exporter.build()?);
    }
    Ok(provider.build())
}

fn build_metrics(
    endpoint: Option<&str>, resource: Resource, export: bool,
) -> Result<SdkMeterProvider> {
    let mut provider = SdkMeterProvider::builder().with_resource(resource);
    if export {
        let mut exporter = MetricExporter::builder().with_tonic();
        if let Some(endpoint) = endpoint {
            exporter = exporter.with_endpoint(endpoint);
        }
        provider = provider.with_periodic_exporter(exporter.build()?);
    }
    Ok(provider.build())
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

    // Pure over their inputs: the process `RUST_LOG` is handed in, never read
    // (other tests run in parallel).
    mod directives {
        use super::super::{LevelFilter, directives};

        #[test]
        fn flag_displaces_bare_level() {
            assert_eq!(
                directives(Some(LevelFilter::DEBUG), LevelFilter::INFO, Some("info")),
                "debug"
            );
            assert_eq!(
                directives(Some(LevelFilter::WARN), LevelFilter::INFO, Some("trace")),
                "warn"
            );
        }

        #[test]
        fn flag_keeps_targets() {
            assert_eq!(
                directives(Some(LevelFilter::DEBUG), LevelFilter::INFO, Some("debug,tower=off")),
                "debug,tower=off"
            );
            assert_eq!(
                directives(Some(LevelFilter::WARN), LevelFilter::INFO, Some("omnia_core=trace")),
                "warn,omnia_core=trace"
            );
        }

        #[test]
        fn no_flag_keeps_rust_log() {
            assert_eq!(
                directives(None, LevelFilter::INFO, Some("warn,omnia_core=debug")),
                "warn,omnia_core=debug"
            );
        }

        #[test]
        fn unset_falls_back() {
            assert_eq!(directives(None, LevelFilter::INFO, None), "info");
            assert_eq!(directives(None, LevelFilter::WARN, Some("")), "warn");
            assert_eq!(directives(Some(LevelFilter::TRACE), LevelFilter::WARN, None), "trace");
        }

        // A targeted `RUST_LOG` with no bare level names what it wants and
        // nothing else; the fallback stands in for an empty variable alone.
        #[test]
        fn targeted_only_takes_no_fallback() {
            assert_eq!(
                directives(None, LevelFilter::INFO, Some("omnia_core=trace")),
                "omnia_core=trace"
            );
        }

        // Every spelling `EnvFilter` reads as a global level is one here, and
        // it renders in the one canonical form; a bare target is not a level.
        #[test]
        fn bare_level_spellings() {
            assert_eq!(directives(None, LevelFilter::INFO, Some("3,tower=off")), "info,tower=off");
            assert_eq!(directives(None, LevelFilter::INFO, Some("DEBUG")), "debug");
            assert_eq!(
                directives(Some(LevelFilter::TRACE), LevelFilter::INFO, Some("INFO")),
                "trace"
            );
            assert_eq!(directives(None, LevelFilter::INFO, Some("tower")), "tower");
        }

        #[test]
        fn invalid_token_dropped() {
            assert_eq!(
                directives(
                    Some(LevelFilter::DEBUG),
                    LevelFilter::INFO,
                    Some("omnia_core=loud,tower=off")
                ),
                "debug,tower=off"
            );
            assert_eq!(directives(None, LevelFilter::INFO, Some("omnia_core=loud")), "info");
        }

        #[test]
        fn whitespace_and_empty_tokens() {
            assert_eq!(
                directives(None, LevelFilter::INFO, Some(" info , tower=off ,,")),
                "info,tower=off"
            );
        }
    }

    mod filter {
        use super::super::filter;

        const MUTES: [&str; 7] = [
            "hyper=off",
            "h2=off",
            "tower=off",
            "tonic=off",
            "opentelemetry=off",
            "opentelemetry_sdk=off",
            "omnia_wasi_otel=off",
        ];

        fn rendered(directives: &str) -> String {
            filter(directives).expect("filter directives parse").to_string()
        }

        fn has(rendered: &str, directive: &str) -> bool {
            rendered.split(',').any(|candidate| candidate == directive)
        }

        #[test]
        fn directives_with_mutes() {
            let rendered = rendered("info,omnia_core=debug");
            for directive in ["info", "omnia_core=debug"].into_iter().chain(MUTES) {
                assert!(has(&rendered, directive), "missing `{directive}` in `{rendered}`");
            }
        }

        #[test]
        fn off() {
            let rendered = rendered("off");
            assert!(has(&rendered, "off"), "{rendered}");
        }

        #[test]
        fn unparsable_directives() {
            assert!(filter("omnia_core=loud").is_err(), "an unknown level must not parse");
        }
    }

    // Pure over its inputs: the environment is a closure, never the process's.
    mod exports {
        use super::super::{Exports, exports};

        fn env(set: &'static [&'static str]) -> impl Fn(&str) -> Option<String> {
            move |name| set.contains(&name).then(|| "http://collector:4317".to_owned())
        }

        #[test]
        fn nothing_configured() {
            assert_eq!(
                exports(None, env(&[])),
                Exports {
                    traces: false,
                    metrics: false
                }
            );
        }

        #[test]
        fn builder_endpoint() {
            assert_eq!(exports(Some("http://collector:4317"), env(&[])), Exports::ALL);
        }

        #[test]
        fn shared_env_endpoint() {
            assert_eq!(exports(None, env(&["OTEL_EXPORTER_OTLP_ENDPOINT"])), Exports::ALL);
        }

        #[test]
        fn per_signal_env_endpoint() {
            assert_eq!(
                exports(None, env(&["OTEL_EXPORTER_OTLP_TRACES_ENDPOINT"])),
                Exports {
                    traces: true,
                    metrics: false
                }
            );
            assert_eq!(
                exports(None, env(&["OTEL_EXPORTER_OTLP_METRICS_ENDPOINT"])),
                Exports {
                    traces: false,
                    metrics: true
                }
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
