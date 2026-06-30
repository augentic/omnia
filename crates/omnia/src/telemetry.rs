//! # Telemetry
//!
//! Host-side OpenTelemetry initialization and OTLP exporters used to report
//! runtime telemetry out-of-the-box.

use std::env;
use std::sync::OnceLock;

use anyhow::{Result, anyhow};
use opentelemetry::trace::TracerProvider;
use opentelemetry::{KeyValue, global};
use opentelemetry_otlp::{MetricExporter, SpanExporter, WithExportConfig};
use opentelemetry_sdk::Resource;
use opentelemetry_sdk::metrics::SdkMeterProvider;
use opentelemetry_sdk::trace::SdkTracerProvider;
use tracing_opentelemetry::MetricsLayer;
use tracing_subscriber::layer::SubscriberExt;
use tracing_subscriber::util::SubscriberInitExt;
use tracing_subscriber::{EnvFilter, Registry};

static RESOURCE: OnceLock<Resource> = OnceLock::new();

const UNKNOWN: &str = "unknown";

/// Telemetry initializer.
pub struct Telemetry {
    /// The name of the application to for the purposes of identifying the
    /// service in telemetry data.
    app_name: String,

    /// The name of the environment, e.g. "production", "staging", "development".
    env_name: Option<String>,

    /// The OpenTelemetry metrics collection endpoint.
    endpoint: Option<String>,
}

impl Telemetry {
    /// Create a new telemetry resource.
    #[must_use]
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            app_name: name.into(),
            env_name: None,
            endpoint: None,
        }
    }

    /// Override the default app name.
    #[must_use]
    pub fn name(mut self, name: impl Into<String>) -> Self {
        self.app_name = name.into();
        self
    }

    /// Set the environment name.
    #[must_use]
    pub fn env(mut self, env_name: impl Into<String>) -> Self {
        self.env_name = Some(env_name.into());
        self
    }

    /// Set the OpenTelemetry endpoint.
    #[must_use]
    pub fn endpoint(mut self, endpoint: impl Into<String>) -> Self {
        self.endpoint = Some(endpoint.into());
        self
    }

    /// Initializes telemetry using the provided configuration.
    ///
    /// # Errors
    ///
    /// Returns an error if the telemetry system fails to initialize, such as if
    /// the OpenTelemetry exporter cannot be created or if setting the global
    /// subscriber fails.
    pub fn build(self) -> Result<()> {
        let resource = Resource::from(&self);
        RESOURCE.set(resource).map_err(|r| anyhow!("failed to set resource: {r:?}"))?;

        // metrics
        let meter_provider = init_metrics(self.endpoint.as_deref())?;
        global::set_meter_provider(meter_provider.clone());

        // tracing
        let tracer_provider = init_traces(self.endpoint.as_deref())?;
        global::set_tracer_provider(tracer_provider.clone());

        let filter_layer = EnvFilter::from_default_env()
            .add_directive("hyper=off".parse()?)
            .add_directive("h2=off".parse()?)
            .add_directive("tonic=off".parse()?);
        // !required for stdout
        let fmt_layer = tracing_subscriber::fmt::layer();
        let tracer = tracer_provider.tracer(self.app_name);
        let tracing_layer = tracing_opentelemetry::layer().with_tracer(tracer);
        let metrics_layer = MetricsLayer::new(meter_provider);

        // set global default subscriber
        Registry::default()
            .with(filter_layer)
            .with(fmt_layer)
            .with(tracing_layer)
            .with(metrics_layer)
            .try_init()?;

        Ok(())
    }
}

fn init_traces(endpoint: Option<&str>) -> Result<SdkTracerProvider> {
    let exporter =
        with_optional_endpoint(SpanExporter::builder().with_tonic(), endpoint).build()?;
    let resource = RESOURCE.wait().clone();

    Ok(SdkTracerProvider::builder().with_resource(resource).with_batch_exporter(exporter).build())
}

fn init_metrics(endpoint: Option<&str>) -> Result<SdkMeterProvider> {
    let exporter =
        with_optional_endpoint(MetricExporter::builder().with_tonic(), endpoint).build()?;
    let resource = RESOURCE.wait().clone();

    Ok(SdkMeterProvider::builder().with_resource(resource).with_periodic_exporter(exporter).build())
}

/// Apply an optional OTLP endpoint override to an exporter builder.
fn with_optional_endpoint<B: WithExportConfig>(builder: B, endpoint: Option<&str>) -> B {
    match endpoint {
        Some(endpoint) => builder.with_endpoint(endpoint),
        None => builder,
    }
}

/// Returns the OpenTelemetry [`Resource`] used to initialize telemetry for a
/// service, or `None` if telemetry has not been initialized.
pub fn resource() -> Option<&'static Resource> {
    RESOURCE.get()
}

impl From<&Telemetry> for Resource {
    fn from(otel: &Telemetry) -> Self {
        Self::builder()
            .with_service_name(otel.app_name.clone())
            .with_attributes(vec![
                KeyValue::new(
                    "deployment.environment",
                    otel.env_name.clone().unwrap_or_else(|| UNKNOWN.to_string()),
                ),
                KeyValue::new("service.namespace", otel.app_name.clone()),
                KeyValue::new("service.version", env!("CARGO_PKG_VERSION")),
                KeyValue::new(
                    "service.instance.id",
                    env::var("HOSTNAME").unwrap_or_else(|_| UNKNOWN.to_string()),
                ),
                KeyValue::new("telemetry.sdk.name", "opentelemetry"),
                KeyValue::new("instrumentation.provider", "opentelemetry"),
            ])
            .build()
    }
}
