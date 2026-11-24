//! Initialise OpenTelemetry

use std::sync::OnceLock;

use anyhow::{Context, Result};
use opentelemetry::global;
use opentelemetry::trace::TracerProvider as _;
use tracing::Metadata;
use tracing_opentelemetry::{MetricsLayer, layer as tracing_layer};
use tracing_subscriber::filter::filter_fn;
use tracing_subscriber::layer::SubscriberExt;
use tracing_subscriber::util::SubscriberInitExt;
use tracing_subscriber::{EnvFilter, Layer as _, Registry};

use crate::otel::{meter, tracer};

static TELEMETRY: OnceLock<Option<Telemetry>> = OnceLock::new();

/// Wrap the provided `inner` function with telemetry support.
///
/// The first `scope` in an instance initializes telemetry before calling the
/// wrapped `inner` function. Exports every span and metric once the returned
/// future completes, as [`flush`] does. On a native target it installs and
/// exports nothing, so the spans `inner` opens reach the caller's subscriber.
///
/// `RUST_LOG` filters console output and spans, falling back to `error` when
/// it is unset. Metrics record whatever it is, whether an instrument's or a
/// `tracing` event's with a `monotonic_counter.`, `counter.`, `histogram.`,
/// or `gauge.` field.
pub async fn scope<F: Future>(inner: impl FnOnce() -> F) -> F::Output {
    let mut owner = false;
    TELEMETRY.get_or_init(|| {
        owner = true;
        if cfg!(not(target_arch = "wasm32")) {
            return None;
        }
        init().inspect_err(report).ok()
    });

    let output = inner().await;
    if owner {
        flush().await;
    }

    output
}

/// Export buffered spans and recorded metrics to the host.
///
/// Export failures are logged, not propagated. Telemetry must not affect
/// application logic. Safe to call when telemetry was never initialized.
pub async fn flush() {
    let Some(telemetry) = telemetry() else { return };
    tracer::export(&telemetry.spans).await;
    meter::export(&telemetry.meters).await;
}

struct Telemetry {
    spans: tracer::SpanBuffer,
    meters: meter::MeterProvider,
}

fn init() -> Result<Telemetry> {
    let console = tracing_subscriber::fmt::layer()
        .with_writer(std::io::stderr)
        .with_filter(filter_fn(|meta| !meta.is_span()));

    let (tracer_provider, spans) = tracer::TracerProvider::new();
    let exporter =
        tracing_layer().with_tracer(tracer_provider.tracer("global")).with_threads(false);

    let meter_provider = meter::MeterProvider::default();

    Registry::default()
        .with(console.and_then(exporter).with_filter(mute(EnvFilter::from_default_env())))
        .with(MetricsLayer::new(meter_provider.clone()).with_filter(filter_fn(metric)))
        .try_init()
        .context("issue initializing subscriber")?;

    global::set_tracer_provider(tracer_provider);
    global::set_meter_provider(meter_provider.clone());

    Ok(Telemetry {
        spans,
        meters: meter_provider,
    })
}

fn mute(filter: EnvFilter) -> EnvFilter {
    filter.add_directive("opentelemetry=off".parse().expect("opentelemetry=off"))
}

fn metric(meta: &Metadata<'_>) -> bool {
    const PREFIXES: [&str; 4] = ["monotonic_counter.", "counter.", "histogram.", "gauge."];
    meta.is_event()
        && meta
            .fields()
            .iter()
            .any(|field| PREFIXES.iter().any(|prefix| field.name().starts_with(prefix)))
}

#[expect(clippy::print_stderr, reason = "the subscriber that would carry this failed to install")]
fn report(err: &anyhow::Error) {
    eprintln!("initialization issue: {err:#}");
}

fn telemetry() -> Option<&'static Telemetry> {
    TELEMETRY.get()?.as_ref()
}
