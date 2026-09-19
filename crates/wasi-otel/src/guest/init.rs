//! Initialise OpenTelemetry

use std::sync::OnceLock;

use anyhow::{Context, Result};
use opentelemetry::global;
use opentelemetry::trace::TracerProvider;
use opentelemetry_sdk::Resource;
use tracing_opentelemetry::{MetricsLayer, layer as tracing_layer};
use tracing_subscriber::filter::{Directive, filter_fn};
use tracing_subscriber::layer::SubscriberExt;
use tracing_subscriber::util::SubscriberInitExt;
use tracing_subscriber::{EnvFilter, Layer as _, Registry, reload};

use crate::guest::generated::omnia::otel::resource;
use crate::guest::{metrics, tracing};

static TELEMETRY: OnceLock<Option<Telemetry>> = OnceLock::new();

const MUTES: [&str; 2] = ["opentelemetry=off", "opentelemetry_sdk=off"];

struct Telemetry {
    filter: reload::Handle<EnvFilter, Registry>,
    spans: tracing::SpanBuffer,
    reader: metrics::Reader,
}

/// Run `f` inside the guest's telemetry lifecycle.
///
/// The first `scope` in an instance initializes telemetry before calling `f`
/// and exports every buffered span and recorded metric once the returned
/// future completes; nested calls are pass-throughs. `#[instrument]` and
/// `command!` route through it, so guests rarely call it directly.
pub async fn scope<F: Future>(inner: impl FnOnce() -> F) -> F::Output {
    let mut owner = false;
    TELEMETRY.get_or_init(|| {
        owner = true;
        init().inspect_err(|error| eprintln!("telemetry not initialized: {error:#}")).ok()
    });

    let output = inner().await;
    if owner {
        flush().await;
    }

    output
}

fn init() -> Result<Telemetry> {
    let resource: Resource = resource::resource().into();

    let (filter_layer, filter) = reload::Layer::new(compose("error")?);
    let fmt_layer = tracing_subscriber::fmt::layer()
        .with_writer(std::io::stderr)
        .with_filter(filter_fn(|meta| !meta.is_span()));

    let (tracer_provider, spans) = tracing::init(resource.clone());
    let tracing_layer =
        tracing_layer().with_tracer(tracer_provider.tracer("global")).with_threads(false);

    let (meter_provider, reader) = metrics::init(resource);
    let metrics_layer = MetricsLayer::new(meter_provider.clone());

    Registry::default()
        .with(filter_layer)
        .with(fmt_layer)
        .with(tracing_layer)
        .with(metrics_layer)
        .try_init()
        .context("issue initializing subscriber")?;

    global::set_tracer_provider(tracer_provider);
    global::set_meter_provider(meter_provider);

    Ok(Telemetry {
        filter,
        spans,
        reader,
    })
}

/// Export buffered spans and recorded metrics to the host.
///
/// Export failures are logged, never propagated: telemetry must not affect
/// application logic. Safe to call when telemetry was never initialized.
pub async fn flush() {
    let Some(telemetry) = telemetry() else { return };
    tracing::export(&telemetry.spans).await;
    metrics::export(&telemetry.reader).await;
}

/// Replace the guest's tracing filter with `directives` refined by `RUST_LOG`.
///
/// `directives` is a `RUST_LOG` string (`info`, `my_guest=debug`). Valid
/// environment directives are applied after it and so win where both select
/// the same target; invalid ones are reported to standard error and ignored.
/// Events after the call follow the new filter, a span already open keeps
/// the verdict it entered under, and the always-on mutes stay in force.
///
/// # Errors
///
/// Returns an error if telemetry is not initialized, `directives` do not
/// parse, or the filter cannot be reloaded.
pub fn set_filter(directives: &str) -> Result<()> {
    let telemetry = telemetry().context("telemetry is not initialized")?;
    telemetry.filter.reload(compose(directives)?).context("issue reloading the filter")
}

fn telemetry() -> Option<&'static Telemetry> {
    TELEMETRY.get()?.as_ref()
}

// The guest's filter: `directives` refined by the valid `RUST_LOG` directives
// (an environment directive replaces a guest directive for the same target),
// with the transport mutes applied last so no directive can reopen them.
fn compose(directives: &str) -> Result<EnvFilter> {
    let rust_log = std::env::var("RUST_LOG").unwrap_or_default();
    let env = rust_log.split(',').filter(|value| !value.is_empty()).filter_map(|value| {
        value
            .parse::<Directive>()
            .inspect_err(|error| eprintln!("ignoring `{value}`: {error}"))
            .ok()
    });
    let mutes = MUTES.into_iter().map(|mute| mute.parse().expect("constant directives parse"));

    Ok(env.chain(mutes).fold(EnvFilter::builder().parse(directives)?, EnvFilter::add_directive))
}
