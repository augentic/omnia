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

// `None` records an attempt that failed: a global subscriber can never be
// replaced, so a retry by every nested `scope` could only fail again.
static TELEMETRY: OnceLock<Option<Telemetry>> = OnceLock::new();

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
    let mut owner = false;
    TELEMETRY.get_or_init(|| {
        owner = true;
        init().inspect_err(|error| eprintln!("telemetry not initialized: {error:#}")).ok()
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
    let Some(Some(telemetry)) = TELEMETRY.get() else { return };
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
    let telemetry =
        TELEMETRY.get().and_then(Option::as_ref).context("telemetry is not initialized")?;
    telemetry.filter.reload(filter(Some(directives))?).context("issue reloading the filter")
}

// Install the subscriber and providers.
fn init() -> Result<Telemetry> {
    let resource: Resource = resource::resource().into();

    let (filter_layer, filter) = reload::Layer::new(filter(None)?);
    // Console tracing goes to stderr: stdout is the guest's semantic output
    // (command-mode pipes and JSON envelopes must stay clean of log lines).
    // Spans are hidden from this layer: one that never sees them prints no
    // span prefix, which would repeat fields such as `correlation_id` on
    // every line. The OpenTelemetry layers below still receive every span.
    let fmt_layer = tracing_subscriber::fmt::layer()
        .with_writer(std::io::stderr)
        .with_filter(filter_fn(|meta| !meta.is_span()));

    let (tracer_provider, spans) = tracing::init(resource.clone());
    // A guest is single-threaded: `thread.id` / `thread.name` attributes
    // would only pad every exported span.
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

    // Publish only after the subscriber installs: a failed `try_init` (e.g.
    // the guest already set a global subscriber) must leave nothing behind.
    global::set_tracer_provider(tracer_provider);
    global::set_meter_provider(meter_provider);
    Ok(Telemetry {
        filter,
        spans,
        reader,
    })
}

// The guest filter: `directives` when given, else `RUST_LOG`. The env arm
// defaults to ERROR so error telemetry is never silently dropped (an empty
// `EnvFilter` disables everything). Both arms mute the OpenTelemetry SDK's
// self-diagnostics, which would otherwise echo through the console layer.
fn filter(directives: Option<&str>) -> Result<EnvFilter> {
    let base = match directives {
        Some(directives) => EnvFilter::builder().parse(directives)?,
        None => {
            EnvFilter::builder().with_default_directive(LevelFilter::ERROR.into()).from_env_lossy()
        }
    };
    Ok(base
        .add_directive("opentelemetry=off".parse()?)
        .add_directive("opentelemetry_sdk=off".parse()?))
}
