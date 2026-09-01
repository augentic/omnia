//! Initialise OpenTelemetry

use std::sync::OnceLock;

use anyhow::{Context, Result, anyhow};
use opentelemetry::trace::TracerProvider;
use opentelemetry::{KeyValue, Value};
use opentelemetry_sdk::Resource;
use opentelemetry_sdk::metrics::SdkMeterProvider;
use opentelemetry_sdk::trace::SdkTracerProvider;
use tracing_opentelemetry::{MetricsLayer, layer as tracing_layer};
use tracing_subscriber::layer::SubscriberExt;
use tracing_subscriber::util::SubscriberInitExt;
use tracing_subscriber::{EnvFilter, Registry};

use crate::guest::generated::omnia::otel::{resource, types};
use crate::guest::{metrics, tracing};

static TRACING: OnceLock<SdkTracerProvider> = OnceLock::new();
static METRICS: OnceLock<SdkMeterProvider> = OnceLock::new();

/// Initialize OpenTelemetry SDK and tracing subscriber.
///
/// # Errors
///
/// Returns an error if the telemetry system fails to initialize, such as if
/// the OpenTelemetry exporter cannot be created or if setting the global
/// subscriber fails.
pub fn init() -> Result<Option<ExitGuard>> {
    if TRACING.get().is_some() || METRICS.get().is_some() {
        return Ok(None);
    }

    let resource: Resource = resource::resource().into();

    // Default to ERROR when `RUST_LOG` is unset so error telemetry is never
    // silently dropped (an empty `EnvFilter` disables everything).
    let filter_layer = EnvFilter::builder()
        .with_default_directive(tracing_subscriber::filter::LevelFilter::ERROR.into())
        .from_env_lossy()
        .add_directive("hyper=off".parse()?)
        .add_directive("h2=off".parse()?)
        .add_directive("tonic=off".parse()?);
    let fmt_layer = tracing_subscriber::fmt::layer();
    let registry = Registry::default().with(filter_layer).with(fmt_layer);

    let tracer_provider = tracing::init(resource.clone());
    let tracing_layer = tracing_layer().with_tracer(tracer_provider.tracer("global"));
    TRACING.set(tracer_provider).map_err(|_e| anyhow!("failed to set tracing provider"))?;

    let meter_provider = metrics::init(resource);
    let metrics_layer = MetricsLayer::new(meter_provider.clone());
    METRICS.set(meter_provider).map_err(|_e| anyhow!("failed to set metrics provider"))?;

    let registry = registry.with(tracing_layer).with(metrics_layer);

    registry.try_init().context("issue initializing subscriber")?;

    Ok(Some(ExitGuard))
}

/// Export buffered spans and recorded metrics to the host.
///
/// Export failures are logged, never propagated: telemetry must not affect
/// application logic. Safe to call when telemetry was never initialized.
pub async fn flush() {
    tracing::flush().await;
    metrics::flush().await;
}

/// [`flush`] if `guard` owns the telemetry lifecycle, disarming it.
///
/// The `#[instrument]` macro awaits this as the outermost instrumented
/// function returns, so telemetry is exported before the surrounding export
/// completes — including ahead of a `wasi:cli/exit` that bypasses `Drop`.
pub async fn flush_guard(guard: Result<Option<ExitGuard>>) {
    if let Ok(Some(guard)) = guard {
        std::mem::forget(guard);
        // The generated `wasi::export` futures are `!Send`; run them as a
        // task on the single-threaded executor and await a `Send` signal so
        // instrumented functions stay usable as `Send` handlers (e.g. axum).
        let (tx, rx) = futures::channel::oneshot::channel();
        wit_bindgen::spawn_local(async move {
            flush().await;
            let _ = tx.send(());
        });
        let _ = rx.await;
    }
}

/// [`ExitGuard`] provides a guard to export telemetry data on drop.
pub struct ExitGuard;

impl Drop for ExitGuard {
    fn drop(&mut self) {
        // `Drop` cannot await and a blocking flush deadlocks when the guard
        // drops as an async export completes (the block wins spawned work,
        // such as a response-body writer, that only finishes after the
        // export returns). Defer the export onto the surrounding
        // component-model task instead; it runs after the export's result
        // is returned and before the task exits.
        wit_bindgen::spawn_local(flush());
    }
}

impl From<types::Resource> for Resource {
    fn from(value: types::Resource) -> Self {
        let attrs = value.attributes.into_iter().map(Into::into).collect::<Vec<_>>();
        let builder = Self::builder();

        if let Some(schema_url) = value.schema_url {
            builder.with_schema_url(attrs, schema_url).build()
        } else {
            builder.with_attributes(attrs).build()
        }
    }
}

impl From<types::KeyValue> for KeyValue {
    fn from(value: types::KeyValue) -> Self {
        Self::new(value.key, value.value)
    }
}

impl From<types::Value> for Value {
    fn from(value: types::Value) -> Self {
        match value {
            types::Value::Bool(v) => Self::Bool(v),
            types::Value::S64(v) => Self::I64(v),
            types::Value::F64(v) => Self::F64(v),
            types::Value::String(v) => Self::String(v.into()),
            types::Value::BoolArray(items) => Self::Array(opentelemetry::Array::Bool(items)),
            types::Value::S64Array(items) => Self::Array(opentelemetry::Array::I64(items)),
            types::Value::F64Array(items) => Self::Array(opentelemetry::Array::F64(items)),
            types::Value::StringArray(items) => Self::Array(opentelemetry::Array::String(
                items.into_iter().map(Into::into).collect(),
            )),
        }
    }
}
