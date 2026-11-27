//! # Telemetry
//!
//! The host's observability stack: the `tracing` subscriber (`fmt` to
//! stderr, with the OTLP span and metric exporters beside it) and the
//! process-wide OpenTelemetry providers they publish. The run's filter is its
//! [`directives`]. It governs the console and the span exporter, never a
//! metric ([`filter`](Telemetry::filter)), and the console alone may refine
//! the runtime's own crates beneath it
//! ([`runtime_level`](Telemetry::runtime_level)). A signal exports only
//! when an OTLP endpoint is configured for it, so a run with no collector
//! exports nothing rather than retrying against one.
//!
//! Guest telemetry handed to [`forward_traces`] and [`forward_metrics`] goes
//! to the host's collector for the signal, over the channel the host's own
//! exporter uses, whichever subscriber the process runs.
//!
//! Telemetry is process-global: the first [`Telemetry::build`] installs the
//! subscriber and the providers, and later builds in the same process are
//! no-ops that reuse the first initialization. Batch exporters and the guest
//! queues hold telemetry, so call [`flush`] before a fast process exit; the
//! runtime does this at the end of every drive.
//!
//! The runtime records its own metrics on OpenTelemetry instruments.
//! [`record_pool`] samples the instance pool and [`record_invocations`] the
//! concurrency gate. [`count_message`], [`count_event`], and [`count_error`]
//! count what the triggers handle. They bind once, when [`Telemetry::build`]
//! settles, to the global meter provider as it then stands. That is omnia's
//! own when the build installs it, and the one the process published
//! otherwise. A metric recorded before then is dropped.

mod collector;

use std::env;
use std::io::IsTerminal;
use std::sync::{Mutex, OnceLock, PoisonError};
use std::time::Duration;

use anyhow::{Result, anyhow};
use opentelemetry::metrics::{Counter, Gauge, Histogram, Meter};
use opentelemetry::trace::TracerProvider;
use opentelemetry::{InstrumentationScope, KeyValue, global};
use opentelemetry_sdk::Resource;
use opentelemetry_sdk::error::{OTelSdkError, OTelSdkResult};
use opentelemetry_sdk::metrics::SdkMeterProvider;
use opentelemetry_sdk::trace::{SdkTracer, SdkTracerProvider};
use tracing::{Metadata, Subscriber};
use tracing_opentelemetry::MetricsLayer;
use tracing_subscriber::filter::{Directive, LevelFilter, ParseError, filter_fn};
use tracing_subscriber::fmt::MakeWriter;
use tracing_subscriber::layer::{Layer, SubscriberExt};
use tracing_subscriber::registry::LookupSpan;
use tracing_subscriber::util::{SubscriberInitExt, TryInitError};
use tracing_subscriber::{EnvFilter, Registry};
use wasmtime::PoolingAllocatorMetrics;

use self::collector::{Flush, Targets, Transport};
pub use self::collector::{Forwarded, forward_metrics, forward_traces};
use crate::Invocations;

static SETTLED: Mutex<bool> = Mutex::new(false);
static RESOURCE: OnceLock<Resource> = OnceLock::new();
static PROVIDERS: OnceLock<Providers> = OnceLock::new();
static METRICS: OnceLock<Metrics> = OnceLock::new();

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
    ///
    /// It wins over every `OTEL_EXPORTER_OTLP_*` endpoint variable. Export
    /// carries no TLS, so an `https://` endpoint fails
    /// [`build`](Self::build), as does a schemeless one unless
    /// `OTEL_EXPORTER_OTLP_INSECURE` is `true`.
    #[must_use]
    pub fn endpoint(mut self, endpoint: impl Into<String>) -> Self {
        let endpoint = endpoint.into();
        self.endpoint = (!endpoint.is_empty()).then_some(endpoint);
        self
    }

    /// Filters the run by `directives` instead of the environment.
    ///
    /// `directives` is a `RUST_LOG` string (`info`, `omnia_core=debug`) —
    /// typically the run's [`directives`], composed from its verbosity flag
    /// and the process `RUST_LOG`. It governs every span and event on the
    /// console and through the span exporter, and the `RUST_LOG` every guest
    /// receives. Metrics record whatever it is, whether an instrument's or a
    /// `tracing` event's with a `monotonic_counter.`, `counter.`,
    /// `histogram.`, or `gauge.` field. The always-on noisy-dependency mutes
    /// still apply.
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
    /// is the console layer's: spans, exported events, and every guest follow
    /// the run's filter, so the span a guest's root spans join stays live
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
    /// The resource and the guests' route to the collector publish whether
    /// or not the subscriber installs, so [`forward_traces`] and
    /// [`forward_metrics`] export beside a subscriber of the process's own.
    ///
    /// # Errors
    ///
    /// Returns an error if the filter directives do not parse, an endpoint is
    /// not a plain-text `http://` URI, or the exporters or the runtime they
    /// run on cannot be started.
    pub fn build(self) -> Result<()> {
        let mut settled = SETTLED.lock().unwrap_or_else(PoisonError::into_inner);
        if *settled {
            return Ok(());
        }

        let directives =
            self.filter.unwrap_or_else(|| directives(None, self.fallback, rust_log().as_deref()));

        // console to stderr, since stdout is the guest's; plain text off a terminal or under `NO_COLOR`
        let console = console(&directives, self.runtime, std::io::stderr, ansi())?;
        let run = filter(&directives)?;
        let level = run.max_level_hint().unwrap_or(LevelFilter::TRACE);

        // one transport for the host's exporters and the guests' queues
        let targets = Targets::resolve(self.endpoint.as_deref(), |name| env::var(name).ok())?;
        let transport = Transport::start(&targets)?;
        let resource = resource_for(&self.name);
        let providers = match Providers::build(&resource, &transport) {
            Ok(providers) => providers,
            Err(error) => {
                transport.shutdown();
                return Err(error);
            }
        };
        let tracer = providers.tracer.tracer(self.name);
        let subscriber = subscriber(run, console, tracer, providers.meter.clone());

        // guest telemetry forwards whichever subscriber the process runs
        transport.publish()?;
        RESOURCE.set(resource).map_err(|_resource| anyhow!("telemetry resource already set"))?;

        // publish the providers only once the subscriber referencing them is installed
        match install(subscriber, level) {
            Ok(()) => {
                providers.publish()?;
                if targets.traces.is_none() || targets.metrics.is_none() {
                    tracing::debug!(
                        traces = targets.traces.is_some(),
                        metrics = targets.metrics.is_some(),
                        "no OTLP endpoint (`OTEL_EXPORTER_OTLP_ENDPOINT`); an unexported \
                         signal is dropped, host and guest alike"
                    );
                }
            }
            Err(error) => {
                tracing::warn!(%error, "a tracing subscriber is already set; omnia's skipped");
            }
        }
        Metrics::bind();
        *settled = true;
        drop(settled);
        Ok(())
    }
}

// read once per build so `directives` stays pure over its inputs
fn rust_log() -> Option<String> {
    env::var("RUST_LOG").ok()
}

struct Providers {
    tracer: SdkTracerProvider,
    meter: SdkMeterProvider,
}

impl Providers {
    // Built whatever exports: guest spans join the tracer's, and a provider
    // with no processor or reader drops what it gets.
    fn build(resource: &Resource, transport: &Transport) -> Result<Self> {
        let mut tracer = SdkTracerProvider::builder().with_resource(resource.clone());
        if let Some(exporter) = transport.span_exporter()? {
            tracer = tracer.with_batch_exporter(exporter);
        }
        let mut meter = SdkMeterProvider::builder().with_resource(resource.clone());
        if let Some(exporter) = transport.metric_exporter()? {
            meter = meter.with_periodic_exporter(exporter);
        }
        Ok(Self {
            tracer: tracer.build(),
            meter: meter.build(),
        })
    }

    fn publish(self) -> Result<()> {
        global::set_meter_provider(self.meter.clone());
        global::set_tracer_provider(self.tracer.clone());
        PROVIDERS.set(self).map_err(|_providers| anyhow!("telemetry providers already installed"))
    }
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

// The metrics layer sits beside the run's filter, not beneath it, so a metric
// event records whatever the run's level. It takes a filter of its own: an
// unfiltered layer would leave every callsite the run disables to be checked
// again at each hit.
fn subscriber<C>(
    run: EnvFilter, console: C, tracer: SdkTracer, meter: SdkMeterProvider,
) -> impl Subscriber + Send + Sync + 'static
where
    C: Layer<Registry> + Send + Sync + 'static,
{
    let exporter = tracing_opentelemetry::layer().with_tracer(tracer);
    Registry::default()
        .with(console.and_then(exporter).with_filter(run))
        .with(MetricsLayer::new(meter).with_filter(filter_fn(metric)))
}

// the events `MetricsLayer` records, by the field prefixes it keeps private
fn metric(meta: &Metadata<'_>) -> bool {
    const PREFIXES: [&str; 4] = ["monotonic_counter.", "counter.", "histogram.", "gauge."];
    meta.is_event()
        && meta
            .fields()
            .iter()
            .any(|field| PREFIXES.iter().any(|prefix| field.name().starts_with(prefix)))
}

// Under `tracing-log`, `try_init` hands `log` the subscriber's level hint,
// which metric events lift to `trace`. A `log` record carries no metric, so
// when `try_init` moves `log`'s level it goes back to the run's. At `trace`,
// every record Cranelift logs would dispatch.
fn install(
    subscriber: impl Subscriber + Send + Sync + 'static, level: LevelFilter,
) -> Result<(), TryInitError> {
    let before = log::max_level();
    subscriber.try_init()?;
    if log::max_level() != before {
        log::set_max_level(match level {
            LevelFilter::OFF => log::LevelFilter::Off,
            LevelFilter::ERROR => log::LevelFilter::Error,
            LevelFilter::WARN => log::LevelFilter::Warn,
            LevelFilter::INFO => log::LevelFilter::Info,
            LevelFilter::DEBUG => log::LevelFilter::Debug,
            _ => log::LevelFilter::Trace,
        });
    }
    Ok(())
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
#[expect(clippy::print_stderr, reason = "the subscriber this composes is not installed yet")]
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

/// Flush batched telemetry to the exporters, and wait for the guests'.
///
/// Guest telemetry queued before the call is waited for at most the
/// signal's export timeout. A no-op when telemetry was never installed.
pub fn flush() {
    let guests = Flush::begin();
    if let Some(providers) = PROVIDERS.get() {
        flush_providers(&providers.tracer, &providers.meter);
    }
    guests.wait();
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

/// Returns the OpenTelemetry [`Resource`] the host's telemetry carries.
///
/// `None` until [`Telemetry::build`] runs, which publishes it whether or not
/// its subscriber installs.
#[must_use]
pub fn resource() -> Option<&'static Resource> {
    RESOURCE.get()
}

/// Records one sample of the instance pool's occupancy on the `pool_*` gauges.
///
/// The gauges are `pool_core_instances`, `pool_component_instances`,
/// `pool_memories`, `pool_tables`, and `pool_stacks`.
pub fn record_pool(sample: &PoolingAllocatorMetrics) {
    if let Some(metrics) = METRICS.get() {
        metrics.pool(sample);
    }
}

/// Records one sample of the concurrency gate on the `invocations_*` gauges.
///
/// The gauges are `invocations_in_flight` and `invocations_waiting`.
pub fn record_invocations(sample: &Invocations) {
    if let Some(metrics) = METRICS.get() {
        metrics.invocations(sample);
    }
}

// one invocation of `service` the gate turned away from `trigger`, on
// `invocations_refused`
pub(crate) fn count_refused(service: &str, trigger: &'static str) {
    if let Some(metrics) = METRICS.get() {
        metrics.refused(service, trigger);
    }
}

// how long one invocation of `service` waited for a slot, on
// `invocations_wait_time`
pub(crate) fn record_wait(service: &str, waited: Duration) {
    if let Some(metrics) = METRICS.get() {
        metrics.wait(service, waited);
    }
}

/// Counts one message delivered to `service` on `message_counter`.
pub fn count_message(service: &str) {
    if let Some(metrics) = METRICS.get() {
        metrics.message(service);
    }
}

/// Counts one websocket event delivered to `service` on `event_counter`.
pub fn count_event(service: &str) {
    if let Some(metrics) = METRICS.get() {
        metrics.event(service);
    }
}

/// Counts one request, message, or event `service` failed to handle on
/// `processing_errors`.
///
/// A failed message delivery passes its `topic`, which the count carries
/// beside `service`. A refusal at the concurrency gate is not a failure.
/// [`Runtime::try_lease`](crate::Runtime::try_lease) counts it on
/// `invocations_refused`.
pub fn count_error(service: &str, topic: Option<&str>) {
    if let Some(metrics) = METRICS.get() {
        metrics.error(service, topic);
    }
}

struct Metrics {
    pool_core_instances: Gauge<u64>,
    pool_component_instances: Gauge<u64>,
    pool_memories: Gauge<u64>,
    pool_tables: Gauge<u64>,
    pool_stacks: Gauge<u64>,
    invocations_in_flight: Gauge<u64>,
    invocations_waiting: Gauge<u64>,
    invocations_refused: Counter<u64>,
    invocations_wait_time: Histogram<f64>,
    message_counter: Counter<u64>,
    event_counter: Counter<u64>,
    processing_errors: Counter<u64>,
}

// in seconds, since the SDK's default boundaries suit milliseconds and would
// put nearly every wait in the first bucket
const WAIT_BOUNDARIES: [f64; 13] =
    [0.0001, 0.001, 0.005, 0.01, 0.025, 0.05, 0.1, 0.25, 0.5, 1.0, 2.5, 5.0, 10.0];

impl Metrics {
    // the global provider once a build settles: omnia's own when it
    // installed, the process's otherwise
    fn bind() {
        let scope =
            InstrumentationScope::builder("omnia").with_version(env!("CARGO_PKG_VERSION")).build();
        METRICS.get_or_init(|| Self::new(&global::meter_with_scope(scope)));
    }

    fn new(meter: &Meter) -> Self {
        let gauge = |name, description| meter.u64_gauge(name).with_description(description).build();
        let counter =
            |name, description| meter.u64_counter(name).with_description(description).build();
        Self {
            pool_core_instances: gauge(
                "pool_core_instances",
                "Core instances allocated from the pool",
            ),
            pool_component_instances: gauge(
                "pool_component_instances",
                "Component instances allocated from the pool",
            ),
            pool_memories: gauge("pool_memories", "Linear memories allocated from the pool"),
            pool_tables: gauge("pool_tables", "Tables allocated from the pool"),
            pool_stacks: gauge("pool_stacks", "Fiber stacks allocated from the pool"),
            invocations_in_flight: gauge(
                "invocations_in_flight",
                "Invocations holding a slot of the concurrency gate",
            ),
            invocations_waiting: gauge(
                "invocations_waiting",
                "Invocations waiting for a slot of the concurrency gate",
            ),
            invocations_refused: counter(
                "invocations_refused",
                "Invocations refused because every slot of the concurrency gate was held",
            ),
            invocations_wait_time: meter
                .f64_histogram("invocations_wait_time")
                .with_description("Time an invocation waited for a slot of the concurrency gate")
                .with_unit("s")
                .with_boundaries(WAIT_BOUNDARIES.to_vec())
                .build(),
            message_counter: counter("message_counter", "Messages delivered to the runtime"),
            event_counter: counter("event_counter", "Websocket events delivered to the runtime"),
            processing_errors: counter(
                "processing_errors",
                "Requests, messages, and events the runtime failed to handle",
            ),
        }
    }

    fn pool(&self, sample: &PoolingAllocatorMetrics) {
        self.pool_core_instances.record(sample.core_instances(), &[]);
        self.pool_component_instances.record(sample.component_instances(), &[]);
        self.pool_memories.record(sample.memories() as u64, &[]);
        self.pool_tables.record(sample.tables() as u64, &[]);
        self.pool_stacks.record(sample.stacks() as u64, &[]);
    }

    fn invocations(&self, sample: &Invocations) {
        self.invocations_in_flight.record(sample.in_flight as u64, &[]);
        self.invocations_waiting.record(sample.waiting as u64, &[]);
    }

    fn refused(&self, service: &str, trigger: &'static str) {
        let attributes =
            [KeyValue::new("service", service.to_owned()), KeyValue::new("trigger", trigger)];
        self.invocations_refused.add(1, &attributes);
    }

    fn wait(&self, service: &str, waited: Duration) {
        let service = KeyValue::new("service", service.to_owned());
        self.invocations_wait_time.record(waited.as_secs_f64(), &[service]);
    }

    fn message(&self, service: &str) {
        self.message_counter.add(1, &[KeyValue::new("service", service.to_owned())]);
    }

    fn event(&self, service: &str) {
        self.event_counter.add(1, &[KeyValue::new("service", service.to_owned())]);
    }

    fn error(&self, service: &str, topic: Option<&str>) {
        let mut attributes = vec![KeyValue::new("service", service.to_owned())];
        attributes.extend(topic.map(|topic| KeyValue::new("topic", topic.to_owned())));
        self.processing_errors.add(1, &attributes);
    }
}

// private rules over the run's directives and environment, which no public
// boundary reaches, and the subscriber `build` installs, whose metrics none
// observes without a collector
#[cfg(test)]
mod tests {
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

    mod refine {
        use super::super::{LevelFilter, refine};

        #[test]
        fn no_runtime_targets() {
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
        fn named_target() {
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
        fn prefix_of_target() {
            assert_eq!(refine("info,omnia=debug", LevelFilter::WARN), "info,omnia=debug");
        }

        // a more specific directive wins where it applies and leaves the rest of
        // the crate to the refinement; a span-only directive names no target
        #[test]
        fn narrower_directives() {
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
    }

    // a metric event records whatever the run's level, which the console keeps to
    mod subscriber {
        use std::io;
        use std::sync::{Arc, Mutex, PoisonError};

        use opentelemetry::trace::TracerProvider as _;
        use opentelemetry_sdk::metrics::data::{ResourceMetrics, ScopeMetrics};
        use opentelemetry_sdk::metrics::{InMemoryMetricExporter, SdkMeterProvider};
        use opentelemetry_sdk::trace::SdkTracerProvider;
        use tracing_subscriber::fmt::MakeWriter;

        use super::super::{console, filter, subscriber};

        #[test]
        fn below_level() {
            let out = Console::default();
            let exporter = InMemoryMetricExporter::default();
            let meter =
                SdkMeterProvider::builder().with_periodic_exporter(exporter.clone()).build();
            let tracer = SdkTracerProvider::builder().build().tracer("test");
            let console = console("warn", None, out.clone(), false).expect("directives parse");
            let run = filter("warn").expect("directives parse");
            let stack = subscriber(run, console, tracer, meter.clone());

            tracing::subscriber::with_default(stack, || {
                tracing::info!(monotonic_counter.published = 1, topic = "orders");
                tracing::debug!(counter.queued = -1);
                tracing::trace!(histogram.latency = 2.5);
                tracing::info!(gauge.depth = 7);
                tracing::warn!("shown");
            });
            meter.force_flush().expect("the provider flushes");

            let finished = exporter.get_finished_metrics().expect("the exporter reads back");
            let mut names: Vec<_> = finished
                .last()
                .into_iter()
                .flat_map(ResourceMetrics::scope_metrics)
                .flat_map(ScopeMetrics::metrics)
                .map(|metric| metric.name().to_owned())
                .collect();
            names.sort();
            assert_eq!(names, ["depth", "latency", "published", "queued"]);

            let shown = out.text();
            assert_eq!(shown.lines().count(), 1, "{shown}");
            assert!(shown.contains("shown"), "{shown}");
        }

        #[derive(Clone, Default)]
        struct Console(Arc<Mutex<Vec<u8>>>);

        impl Console {
            fn text(&self) -> String {
                let bytes = self.0.lock().unwrap_or_else(PoisonError::into_inner);
                String::from_utf8_lossy(&bytes).into_owned()
            }
        }

        impl io::Write for Console {
            fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
                self.0.lock().unwrap_or_else(PoisonError::into_inner).write(buf)
            }

            fn flush(&mut self) -> io::Result<()> {
                Ok(())
            }
        }

        impl<'a> MakeWriter<'a> for Console {
            type Writer = Self;

            fn make_writer(&'a self) -> Self::Writer {
                self.clone()
            }
        }
    }
}
