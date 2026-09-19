//! Initialise OpenTelemetry

use std::sync::OnceLock;

use anyhow::{Context, Result};
use opentelemetry::global;
use opentelemetry::trace::TracerProvider;
use opentelemetry_sdk::Resource;
use tracing_opentelemetry::{MetricsLayer, layer as tracing_layer};
use tracing_subscriber::filter::{LevelFilter, filter_fn};
use tracing_subscriber::layer::SubscriberExt;
use tracing_subscriber::util::SubscriberInitExt;
use tracing_subscriber::{EnvFilter, Layer as _, Registry, reload};

use crate::guest::generated::omnia::otel::resource;
use crate::guest::{metrics, tracing};

static TELEMETRY: OnceLock<Telemetry> = OnceLock::new();

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
pub async fn scope<F: Future>(f: impl FnOnce() -> F) -> F::Output {
    let owner = init().unwrap_or_else(|error| {
        eprintln!("telemetry not initialized: {error:#}");
        false
    });
    // `f` runs after `init` so `span!` sees a live dispatcher; awaited by
    // value so the instrumented future (and its span) drops before the export.
    let output = f().await;
    if owner {
        flush().await;
    }
    output
}

/// Export buffered spans and recorded metrics to the host.
///
/// Export failures are logged, never propagated: telemetry must not affect
/// application logic. Safe to call when telemetry was never initialized.
pub async fn flush() {
    let Some(telemetry) = TELEMETRY.get() else { return };
    tracing::export(&telemetry.spans).await;
    metrics::export(&telemetry.reader).await;
}

/// Replace the guest's tracing filter with `directives`, a `RUST_LOG` string.
///
/// Events after the call follow the new filter. A span already open keeps
/// the verdict it entered under. The always-on mutes stay in force.
///
/// # Errors
///
/// Returns an error if telemetry is not initialized or `directives` do not
/// parse.
pub fn set_filter(directives: &str) -> Result<()> {
    let telemetry = TELEMETRY.get().context("telemetry is not initialized")?;
    telemetry.filter.reload(filter(Some(directives))?).context("issue reloading the filter")
}

// Install the subscriber and providers; `Ok(true)` when this call did so.
fn init() -> Result<bool> {
    if TELEMETRY.get().is_some() {
        return Ok(false);
    }

    let resource: Resource = resource::resource().into();

    let (filter_layer, filter) = reload::Layer::new(filter(None)?);
    // Hide spans from the console layer: one that never sees them prints no
    // span prefix, which would repeat fields such as `correlation_id` on
    // every line. The OpenTelemetry layers below still receive every span.
    let fmt_layer = tracing_subscriber::fmt::layer().with_filter(filter_fn(|meta| !meta.is_span()));

    let (tracer_provider, spans) = tracing::init(resource.clone());
    let tracing_layer = tracing_layer().with_tracer(tracer_provider.tracer("global"));

    let (meter_provider, reader) = metrics::init(resource);
    let metrics_layer = MetricsLayer::new(meter_provider.clone());

    Registry::default()
        .with(filter_layer)
        .with(fmt_layer)
        .with(tracing_layer)
        .with(metrics_layer)
        .try_init()
        .context("issue initializing subscriber")?;

    // Publish only after the subscriber installs: a failed `try_init` (e.g.
    // the guest already set a global subscriber) must leave nothing behind.
    global::set_tracer_provider(tracer_provider);
    global::set_meter_provider(meter_provider);
    Ok(TELEMETRY
        .set(Telemetry {
            filter,
            spans,
            reader,
        })
        .is_ok())
}

// The guest filter: `directives` when given, else `RUST_LOG`. The env arm
// defaults to ERROR so error telemetry is never silently dropped (an empty
// `EnvFilter` disables everything). Both arms carry the host's always-on
// mutes: transports and the OpenTelemetry SDK's self-diagnostics.
fn filter(directives: Option<&str>) -> Result<EnvFilter> {
    let base = match directives {
        Some(directives) => EnvFilter::builder().parse(directives)?,
        None => {
            EnvFilter::builder().with_default_directive(LevelFilter::ERROR.into()).from_env_lossy()
        }
    };
    Ok(base
        .add_directive("hyper=off".parse()?)
        .add_directive("h2=off".parse()?)
        .add_directive("tonic=off".parse()?)
        .add_directive("opentelemetry=off".parse()?)
        .add_directive("opentelemetry_sdk=off".parse()?))
}
