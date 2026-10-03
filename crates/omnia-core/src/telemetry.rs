//! # Telemetry
//!
//! The host's observability stack: the `tracing` subscriber (the run's
//! `EnvFilter`, `fmt` to stderr) with the OTLP span and metric exporters
//! layered beneath it, and the process-wide OpenTelemetry providers they
//! publish. The run's filter is its [`directives`]; the console alone may
//! refine the runtime's own crates beneath it
//! ([`runtime_level`](Telemetry::runtime_level)). A signal's exporter
//! attaches only when an OTLP endpoint is configured for it, so a run with no
//! collector exports nothing rather than retrying against one.
//!
//! Telemetry is process-global: the first [`Telemetry::build`] installs the
//! subscriber and the providers, and later builds in the same process are
//! no-ops that reuse the first initialization. Batch exporters queue
//! telemetry, so call [`flush`] before a fast process exit; the runtime does
//! this at the end of every drive.

use std::env;
use std::io::IsTerminal;
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
use tracing::Subscriber;
use tracing_opentelemetry::MetricsLayer;
use tracing_subscriber::filter::{Directive, LevelFilter, ParseError};
use tracing_subscriber::fmt::MakeWriter;
use tracing_subscriber::layer::{Layer, SubscriberExt};
use tracing_subscriber::registry::LookupSpan;
use tracing_subscriber::util::SubscriberInitExt;
use tracing_subscriber::{EnvFilter, Registry};

static SETTLED: Mutex<bool> = Mutex::new(false);
static PROVIDERS: OnceLock<Providers> = OnceLock::new();

/// The runtime's own crate prefixes, which the console may refine apart from
/// the run — see [`Telemetry::runtime_level`].
///
/// `omnia::` matches only the composition root, never the bare `omnia` prefix
/// capability crates share (`omnia_sdk`, `omnia_wasi_http`), which a directive
/// would match by prefix.
pub const RUNTIME: [&str; 4] = ["omnia::", "omnia_core", "omnia_link", "omnia_plugin"];

/// Builder for the host's telemetry: the `tracing` subscriber with OTLP
/// exporters beneath it.
pub struct Telemetry {
    name: String,
    endpoint: Option<String>,
    filter: Option<String>,
    fallback: LevelFilter,
    runtime: Option<LevelFilter>,
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
            runtime: None,
        }
    }

    /// Sets the OTLP gRPC endpoint both signals export to; empty is unset.
    #[must_use]
    pub fn endpoint(mut self, endpoint: impl Into<String>) -> Self {
        // empty would override the exporter's own `OTEL_EXPORTER_OTLP_*` resolution
        let endpoint = endpoint.into();
        self.endpoint = (!endpoint.is_empty()).then_some(endpoint);
        self
    }

    /// Filters the run by `directives` instead of the environment.
    ///
    /// `directives` is a `RUST_LOG` string (`info`, `omnia_core=debug`) —
    /// typically the run's [`directives`], composed from its verbosity flag
    /// and the process `RUST_LOG`. It governs every span and event: the
    /// console, the exporters, and the `RUST_LOG` every guest receives. The
    /// always-on noisy-dependency mutes still apply.
    #[must_use]
    pub fn filter(mut self, directives: impl Into<String>) -> Self {
        self.filter = Some(directives.into());
        self
    }

    /// Sets the level the run falls back to when the environment sets no
    /// `RUST_LOG`.
    ///
    /// `WARN` when not called. Explicit [`filter`](Self::filter) directives
    /// take precedence over both.
    #[must_use]
    pub const fn fallback(mut self, level: LevelFilter) -> Self {
        self.fallback = level;
        self
    }

    /// Shows the runtime's own crates on the console at `level` alone.
    ///
    /// The console's filter is the run's with each of [`RUNTIME`] refined to
    /// `level`; a directive the run's filter already carries for one of them
    /// (`omnia_core=trace`, `omnia=debug`) stands as written. The refinement
    /// is the console layer's: spans, the exporters, and every guest follow
    /// the run's filter, so the span guest telemetry grafts onto stays live
    /// whatever the console shows. `None` leaves the console at the run's
    /// filter, as does not calling.
    #[must_use]
    pub fn runtime_level(mut self, level: impl Into<Option<LevelFilter>>) -> Self {
        self.runtime = level.into();
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

        let directives =
            self.filter.unwrap_or_else(|| directives(None, self.fallback, rust_log().as_deref()));

        let exports = exports(self.endpoint.as_deref(), |name| env::var(name).ok());
        let providers = Providers::build(&self.name, self.endpoint.as_deref(), exports)?;
        let tracer = providers.tracer.tracer(self.name);

        // console to stderr, since stdout is the guest's; plain text off a terminal or under `NO_COLOR`
        let console = console(&directives, self.runtime, std::io::stderr, ansi())?;

        // every layer filtered: an unfiltered one beside the console's per-layer
        // filter drops the level hint, and `log` records flood in unfiltered
        let subscriber = Registry::default()
            .with(filter(&directives)?)
            .with(console)
            .with(
                tracing_opentelemetry::layer()
                    .with_tracer(tracer)
                    .with_filter(filter(&directives)?),
            )
            .with(MetricsLayer::new(providers.meter.clone()).with_filter(filter(&directives)?));

        // publish the providers only once the subscriber referencing them is installed
        match subscriber.try_init() {
            Ok(()) => {
                providers.publish()?;
                if exports != Exports::ALL {
                    tracing::debug!(
                        traces = exports.traces,
                        metrics = exports.metrics,
                        "no OTLP endpoint (`OTEL_EXPORTER_OTLP_ENDPOINT`); an unexported \
                         signal is dropped"
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
    fn set(value: Option<impl AsRef<str>>) -> bool {
        value.is_some_and(|value| !value.as_ref().is_empty())
    }
    let shared = set(endpoint) || set(env(OTEL_EXPORTER_OTLP_ENDPOINT));

    Exports {
        traces: shared || set(env(OTEL_EXPORTER_OTLP_TRACES_ENDPOINT)),
        metrics: shared || set(env(OTEL_EXPORTER_OTLP_METRICS_ENDPOINT)),
    }
}

/// The run's tracing directives: the verbosity flag composed with `RUST_LOG`.
///
/// The global level is `level` when one is selected, else the bare level
/// `rust_log` carries, else `fallback` when it carries no directive at all;
/// every targeted directive (`tower=off`, `omnia_core=trace`, `[span]=debug`)
/// in `rust_log` follows it.
///
/// A selected level displaces only `rust_log`'s bare level, so a flag steps
/// the run's level without discarding the operator's refinements. A token
/// that is neither is reported on stderr and dropped, as `EnvFilter` drops
/// it on a bare run; the result always parses.
#[must_use]
pub fn directives(
    level: Option<LevelFilter>, fallback: LevelFilter, rust_log: Option<&str>,
) -> String {
    let tokens = tokens(rust_log);
    for (token, error) in &tokens.invalid {
        eprintln!("ignoring `RUST_LOG` directive `{token}`: {error}");
    }

    tokens
        .bare(level, fallback)
        .map(|level| level.to_string())
        .into_iter()
        .chain(tokens.targeted.iter().map(|&token| token.to_owned()))
        .collect::<Vec<_>>()
        .join(",")
}

/// The bare level the run's directives lead with.
///
/// `level` when one is selected, else the bare level `rust_log` carries,
/// else `fallback` when it carries no directive at all — the level
/// [`directives`] renders first over the same inputs — and `None` when
/// `rust_log` names targets alone, so no bare level governs the run.
#[must_use]
pub fn bare(
    level: Option<LevelFilter>, fallback: LevelFilter, rust_log: Option<&str>,
) -> Option<LevelFilter> {
    tokens(rust_log).bare(level, fallback)
}

// the refinement is per-layer so it never disables a span
fn console<S, W>(
    directives: &str, runtime: Option<LevelFilter>, writer: W, ansi: bool,
) -> Result<impl Layer<S>>
where
    S: Subscriber + for<'a> LookupSpan<'a> + 'static,
    W: for<'w> MakeWriter<'w> + 'static,
{
    let refinement =
        runtime.map(|level| EnvFilter::builder().parse(refine(directives, level))).transpose()?;
    Ok(tracing_subscriber::fmt::layer().with_writer(writer).with_ansi(ansi).with_filter(refinement))
}

// `directives` plus `<prefix>=<level>` for each of `RUNTIME` no directive
// already governs — by naming it or a prefix of it (`omnia_core=trace`,
// `omnia=debug`)
fn refine(directives: &str, level: LevelFilter) -> String {
    let named: Vec<&str> = directives.split(',').filter_map(target).collect();
    let governed = |prefix: &str| named.iter().any(|target| prefix.starts_with(target));
    let refinements = RUNTIME
        .into_iter()
        .filter(|prefix| !governed(prefix))
        .map(|prefix| format!(",{prefix}={level}"));
    std::iter::once(directives.to_owned()).chain(refinements).collect()
}

// The target a directive governs by prefix; a bare level or a span-only
// directive (`[cli-run]=info`) governs none
fn target(directive: &str) -> Option<&str> {
    let directive = directive.trim();
    if directive.parse::<LevelFilter>().is_ok() {
        return None;
    }
    let target = directive.split(['=', '[']).next().unwrap_or_default().trim();
    (!target.is_empty()).then_some(target)
}

// The tokens of a `RUST_LOG`: its bare level (the last, as `EnvFilter`
// reads one), its targeted directives, and the tokens that are neither.
struct Tokens<'a> {
    global: Option<LevelFilter>,
    targeted: Vec<&'a str>,
    invalid: Vec<(&'a str, ParseError)>,
}

impl Tokens<'_> {
    // the fallback fills an empty `RUST_LOG`, never one that names a target
    fn bare(&self, level: Option<LevelFilter>, fallback: LevelFilter) -> Option<LevelFilter> {
        level.or(self.global).or_else(|| self.targeted.is_empty().then_some(fallback))
    }
}

fn tokens(rust_log: Option<&str>) -> Tokens<'_> {
    let mut tokens = Tokens {
        global: None,
        targeted: Vec::new(),
        invalid: Vec::new(),
    };
    for token in rust_log.unwrap_or_default().split(',').map(str::trim) {
        // an empty token would parse as `error`; `EnvFilter` skips it too
        if token.is_empty() {
            continue;
        }
        if let Ok(level) = token.parse::<LevelFilter>() {
            tokens.global = Some(level);
        } else if let Err(error) = token.parse::<Directive>() {
            tokens.invalid.push((token, error));
        } else {
            tokens.targeted.push(token);
        }
    }
    tokens
}

// read once per build so `directives` stays pure over its inputs
fn rust_log() -> Option<String> {
    env::var("RUST_LOG").ok()
}

/// Whether the host console colours its lines: stderr is a terminal and
/// `NO_COLOR` is unset or empty.
///
/// Decided once, for the console layer and for every guest's environment.
/// `with_ansi` overrides the `fmt` default that honours `NO_COLOR` on a
/// terminal, so the console reads the variable itself.
pub(crate) fn ansi() -> bool {
    colour(std::io::stderr().is_terminal(), env::var("NO_COLOR").ok().as_deref())
}

// the `NO_COLOR` convention: any non-empty value disables colour, an empty one does not
fn colour(terminal: bool, no_color: Option<&str>) -> bool {
    terminal && no_color.is_none_or(str::is_empty)
}

// `directives` plus the noisy-dependency mutes; `tower` is muted whole since
// it narrates every buffered request through `reqwest` as well as `tonic`
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

struct Providers {
    resource: Resource,
    tracer: SdkTracerProvider,
    meter: SdkMeterProvider,
}

impl Providers {
    // Built whatever exports: guest telemetry grafts onto the resource and
    // tracer, and a provider with no processor or reader drops what it gets.
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

// force-flush rather than shut down, so export continues and repeated flushes are safe
fn flush_providers(tracer: &SdkTracerProvider, meter: &SdkMeterProvider) {
    settle("traces", tracer.force_flush());
    settle("metrics", meter.force_flush());
}

// an already-shut-down provider has nothing left to flush
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

// these pin the tracing/OTLP SDK contract, not guest-host boundary behaviour
#[cfg(test)]
mod tests {
    use std::sync::{Arc, Mutex};

    use opentelemetry::trace::{Tracer as _, TracerProvider as _};
    use opentelemetry_sdk::metrics::SdkMeterProvider;
    use opentelemetry_sdk::trace::{SdkTracerProvider, SpanData, SpanExporter};
    use tracing::Subscriber as _;
    use tracing_opentelemetry::MetricsLayer;
    use tracing_subscriber::layer::{Layer as _, SubscriberExt as _};

    use super::{LevelFilter, OTelSdkResult, Registry, console, filter, flush_providers};

    // keeps its records, so flushing is observable without a collector
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

    // captures the console, so what a refinement shows is observable
    #[derive(Clone, Debug, Default)]
    struct Console(Arc<Mutex<Vec<u8>>>);

    impl Console {
        fn text(&self) -> String {
            String::from_utf8(self.0.lock().expect("console lock").clone()).expect("utf-8 console")
        }
    }

    impl std::io::Write for Console {
        fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
            self.0.lock().expect("console lock").extend_from_slice(buf);
            Ok(buf.len())
        }

        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }

    impl<'a> super::MakeWriter<'a> for Console {
        type Writer = Self;

        fn make_writer(&'a self) -> Self {
            self.clone()
        }
    }

    // batch-exported, so spans stay queued until a flush pushes them
    fn providers(exporter: &Recording) -> (SdkTracerProvider, SdkMeterProvider) {
        (
            SdkTracerProvider::builder().with_batch_exporter(exporter.clone()).build(),
            SdkMeterProvider::builder().build(),
        )
    }

    #[test]
    fn empty_endpoint_unset() {
        use super::Telemetry;

        assert!(Telemetry::new("svc").endpoint("").endpoint.is_none());
        assert_eq!(
            Telemetry::new("svc").endpoint("http://collector:4317").endpoint.as_deref(),
            Some("http://collector:4317")
        );
    }

    // the process `RUST_LOG` is handed in, never read: other tests run in parallel
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

        // the fallback stands in for an empty variable alone
        #[test]
        fn targeted_only_takes_no_fallback() {
            assert_eq!(
                directives(None, LevelFilter::INFO, Some("omnia_core=trace")),
                "omnia_core=trace"
            );
        }

        // every spelling `EnvFilter` reads as a level renders canonically; a bare target is not one
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

    // `NO_COLOR` is honoured on a terminal by its convention — any non-empty
    // value disables colour, an empty one does not — and off a terminal
    // nothing enables it
    mod colour {
        use super::super::colour;

        #[test]
        fn terminal() {
            assert!(colour(true, None));
            assert!(colour(true, Some("")));
            assert!(!colour(true, Some("1")));
            assert!(!colour(true, Some("0")));
        }

        #[test]
        fn piped() {
            assert!(!colour(false, None));
            assert!(!colour(false, Some("")));
        }
    }

    // the level `directives` leads with, over the same composition
    mod bare {
        use super::super::{LevelFilter, bare};

        #[test]
        fn flag_displaces_bare_level() {
            assert_eq!(
                bare(Some(LevelFilter::INFO), LevelFilter::WARN, Some("trace")),
                Some(LevelFilter::INFO)
            );
        }

        #[test]
        fn no_flag_keeps_rust_log() {
            assert_eq!(
                bare(None, LevelFilter::WARN, Some("debug,omnia_core=trace")),
                Some(LevelFilter::DEBUG)
            );
        }

        #[test]
        fn unset_falls_back() {
            assert_eq!(bare(None, LevelFilter::WARN, None), Some(LevelFilter::WARN));
            assert_eq!(
                bare(None, LevelFilter::WARN, Some("omnia_core=loud")),
                Some(LevelFilter::WARN)
            );
        }

        #[test]
        fn targeted_only_takes_no_fallback() {
            assert_eq!(bare(None, LevelFilter::WARN, Some("my_sdk=debug")), None);
        }
    }

    mod refine {
        use super::super::{LevelFilter, RUNTIME, refine};

        #[test]
        fn refines_runtime_crates() {
            assert_eq!(
                refine("info", LevelFilter::WARN),
                "info,omnia::=warn,omnia_core=warn,omnia_link=warn,omnia_plugin=warn"
            );
            assert_eq!(
                refine("debug,tower=off", LevelFilter::INFO),
                "debug,tower=off,omnia::=info,omnia_core=info,omnia_link=info,omnia_plugin=info"
            );
        }

        // a directive of the operator's governing a runtime target stands as written
        #[test]
        fn named_target_stands() {
            assert_eq!(
                refine("info,omnia_core=trace", LevelFilter::WARN),
                "info,omnia_core=trace,omnia::=warn,omnia_link=warn,omnia_plugin=warn"
            );
            assert_eq!(
                refine("info,omnia_link[dispatch]=trace", LevelFilter::WARN),
                "info,omnia_link[dispatch]=trace,omnia::=warn,omnia_core=warn,omnia_plugin=warn"
            );
        }

        // `omnia=debug` matches every runtime target by prefix, so refining
        // one beneath it would override what the operator wrote
        #[test]
        fn prefix_of_target_stands() {
            assert_eq!(refine("info,omnia=debug", LevelFilter::WARN), "info,omnia=debug");
        }

        // a more specific directive wins where it applies and leaves the rest of
        // the crate to the refinement; a span-only directive names no target
        #[test]
        fn narrower_directives_refined_around() {
            assert_eq!(
                refine("info,omnia_core::runtime=trace", LevelFilter::WARN),
                "info,omnia_core::runtime=trace,omnia::=warn,omnia_core=warn,omnia_link=warn,\
                 omnia_plugin=warn"
            );
            assert_eq!(
                refine("info,[cli-run]=trace", LevelFilter::WARN),
                "info,[cli-run]=trace,omnia::=warn,omnia_core=warn,omnia_link=warn,omnia_plugin=warn"
            );
        }

        // the composition root's prefix leaves `omnia_cursor`, `omnia_sdk`,
        // and every `omnia_wasi_*` host to the run's level
        #[test]
        fn runtime_prefixes() {
            for prefix in RUNTIME {
                assert!(prefix.starts_with("omnia"), "{prefix}");
                for other in ["omnia_cursor", "omnia_sdk", "omnia_wasi_http", "omnia"] {
                    assert!(!other.starts_with(prefix), "`{prefix}` would govern `{other}`");
                }
            }
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

        // `omnia::` is a target `EnvFilter` accepts, so the refinement parses whole
        #[test]
        fn refined() {
            use super::super::{LevelFilter, refine};

            let rendered = rendered(&refine("info,tower=off", LevelFilter::WARN));
            for directive in ["info", "omnia::=warn", "omnia_core=warn", "omnia_plugin=warn"] {
                assert!(has(&rendered, directive), "missing `{directive}` in `{rendered}`");
            }
        }

        #[test]
        fn unparsable_directives() {
            assert!(filter("omnia_core=loud").is_err(), "an unknown level must not parse");
        }
    }

    // the environment is a closure, never the process's
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

        // an `=` left in a profile attaches nothing
        #[test]
        fn empty_is_unset() {
            let empty = |_: &str| Some(String::new());
            assert_eq!(
                exports(Some(""), empty),
                Exports {
                    traces: false,
                    metrics: false
                }
            );
            assert_eq!(
                exports(Some(""), env(&["OTEL_EXPORTER_OTLP_TRACES_ENDPOINT"])),
                Exports {
                    traces: true,
                    metrics: false
                }
            );
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

    // The refinement is the console's alone: at a bare command run it hides
    // the runtime's `info` while an `info` span of the same crate — the one
    // guest telemetry grafts onto — is live and exports.
    #[test]
    fn refined_spans() {
        let out = Console::default();
        let exporter = Recording::default();
        let (tracer, meter) = providers(&exporter);
        let subscriber = Registry::default()
            .with(filter("info").expect("directives parse"))
            .with(console("info", Some(LevelFilter::WARN), out.clone(), false).expect("parse"))
            .with(tracing_opentelemetry::layer().with_tracer(tracer.tracer("test")));

        tracing::subscriber::with_default(subscriber, || {
            let anchor = tracing::info_span!("cli-run");
            assert!(!anchor.is_disabled(), "the run's filter governs spans");
            let _entered = anchor.enter();
            tracing::info!("guest loaded");
            tracing::warn!("mount failed");
            tracing::info!(target: "omnia_cursor::model", "completion");
        });

        flush_providers(&tracer, &meter);
        assert_eq!(exporter.names(), ["cli-run"]);
        let console = out.text();
        assert!(!console.contains("guest loaded"), "{console}");
        assert!(console.contains("mount failed"), "{console}");
        assert!(console.contains("completion"), "{console}");
    }

    // The stack `build` installs, as `tracing-log` reads it: the hint is the
    // run's level, refined or not, so `log` records above it never dispatch.
    #[test]
    fn stack_level_hint() {
        for runtime in [Some(LevelFilter::WARN), None] {
            let exporter = Recording::default();
            let (tracer, meter) = providers(&exporter);
            let stack = Registry::default()
                .with(filter("info,tower=off").expect("parse"))
                .with(console("info,tower=off", runtime, Console::default(), false).expect("parse"))
                .with(
                    tracing_opentelemetry::layer()
                        .with_tracer(tracer.tracer("test"))
                        .with_filter(filter("info,tower=off").expect("parse")),
                )
                .with(
                    MetricsLayer::new(meter).with_filter(filter("info,tower=off").expect("parse")),
                );
            assert_eq!(stack.max_level_hint(), Some(LevelFilter::INFO), "runtime {runtime:?}");
        }
    }

    // the fast-exit contract: a span emitted just before a flush reaches the exporter
    #[test]
    fn flush_exports() {
        let exporter = Recording::default();
        let (tracer, meter) = providers(&exporter);

        tracer.tracer("test").start("first-drive");
        assert_eq!(exporter.names(), Vec::<String>::new());

        flush_providers(&tracer, &meter);
        assert_eq!(exporter.names(), ["first-drive"]);

        tracer.tracer("test").start("second-drive");
        flush_providers(&tracer, &meter);
        assert_eq!(exporter.names(), ["first-drive", "second-drive"]);
    }
}
