//! Initialise OpenTelemetry

use std::sync::OnceLock;

use anyhow::{Context, Result};
use opentelemetry::global;
use opentelemetry::trace::TracerProvider as _;
use tracing_opentelemetry::{MetricsLayer, layer as tracing_layer};
use tracing_subscriber::filter::filter_fn;
use tracing_subscriber::layer::SubscriberExt;
use tracing_subscriber::util::SubscriberInitExt;
use tracing_subscriber::{EnvFilter, Layer as _, Registry};

use crate::guest::generated::omnia::otel::{resource, types};
use crate::guest::{metrics, tracing};

static TELEMETRY: OnceLock<Option<Telemetry>> = OnceLock::new();

/// Wrap the provided `inner` function with telemetry support.
///
/// The first `scope` in an instance initializes telemetry before calling the
/// wrapped `inner` function: the subscriber follows `RUST_LOG`, falling back
/// to `error` when it is unset. Exports every span and metric once the
/// returned future completes.
pub async fn scope<F: Future>(inner: impl FnOnce() -> F) -> F::Output {
    let mut owner = false;
    TELEMETRY.get_or_init(|| {
        owner = true;
        init().inspect_err(|err| eprintln!("initialization issue: {err:#}")).ok()
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
    tracing::export(&telemetry.spans).await;
    metrics::export(&telemetry.meters, &telemetry.resource).await;
}

struct Telemetry {
    spans: tracing::SpanBuffer,
    meters: metrics::MeterProvider,
    resource: types::Resource,
}

fn init() -> Result<Telemetry> {
    let resource = resource::resource();

    let filter_layer = mute(EnvFilter::from_default_env());
    let fmt_layer = tracing_subscriber::fmt::layer()
        .with_writer(std::io::stderr)
        .with_filter(filter_fn(|meta| !meta.is_span()));

    let (tracer_provider, spans) = tracing::TracerProvider::new();
    let tracing_layer =
        tracing_layer().with_tracer(tracer_provider.tracer("global")).with_threads(false);

    let meter_provider = metrics::MeterProvider::default();
    let metrics_layer = MetricsLayer::new(meter_provider.clone());

    Registry::default()
        .with(filter_layer)
        .with(fmt_layer)
        .with(tracing_layer)
        .with(metrics_layer)
        .try_init()
        .context("issue initializing subscriber")?;

    global::set_tracer_provider(tracer_provider);
    global::set_meter_provider(meter_provider.clone());

    Ok(Telemetry {
        spans,
        meters: meter_provider,
        resource,
    })
}

fn mute(filter: EnvFilter) -> EnvFilter {
    filter.add_directive("opentelemetry=off".parse().expect("opentelemetry=off"))
}

fn telemetry() -> Option<&'static Telemetry> {
    TELEMETRY.get()?.as_ref()
}
