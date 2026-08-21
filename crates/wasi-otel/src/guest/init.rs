//! Initialise OpenTelemetry

use std::sync::OnceLock;

use anyhow::{Context, Result, anyhow};
use opentelemetry::trace::TracerProvider;
use opentelemetry::{KeyValue, Value};
use opentelemetry_sdk::Resource;
use opentelemetry_sdk::error::OTelSdkError;
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

    let filter_layer = EnvFilter::from_default_env()
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

/// Flush and shut down the OpenTelemetry SDK.
///
/// Call this before a process exit that bypasses `Drop` (for example
/// `wasi:cli/exit`). Safe to call when telemetry was never initialized
/// or has already been shut down.
pub fn shutdown() {
    if let Some(tracer_provider) = TRACING.get() {
        match tracer_provider.shutdown() {
            Ok(()) | Err(OTelSdkError::AlreadyShutdown) => (),
            Err(e) => ::tracing::error!("failed to export tracing: {e}"),
        }
    }
    if let Some(meter_provider) = METRICS.get() {
        match meter_provider.shutdown() {
            Ok(()) | Err(OTelSdkError::AlreadyShutdown) => (),
            Err(e) => ::tracing::error!("failed to export metrics: {e}"),
        }
    }
}

/// [`ExitGuard`] provides a guard to export telemetry data on drop.
pub struct ExitGuard;

impl Drop for ExitGuard {
    fn drop(&mut self) {
        shutdown();
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
