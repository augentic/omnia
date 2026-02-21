//! # Export
//!
//! Exports spans and metrics to an OpenTelemetry collector.

use std::sync::Arc;
use std::time::Duration;

use anyhow::Result;
use opentelemetry_otlp::{MetricExporter, SpanExporter};
use opentelemetry_sdk::Resource;
use opentelemetry_sdk::error::OTelSdkError;
use opentelemetry_sdk::metrics::Temporality;
use opentelemetry_sdk::metrics::data::ResourceMetrics;
use opentelemetry_sdk::metrics::exporter::PushMetricExporter;
use opentelemetry_sdk::trace::{SpanData, SpanExporter as SpanExporterTrait};
use rand::Rng;
use tokio::sync::Mutex;
use tokio::task::block_in_place;
use tokio::time;

/// Add retry logic to a `SpanExporter`.
#[derive(Debug, Clone)]
pub struct RetrySpanExporter {
    inner: Arc<Mutex<SpanExporter>>,
    backoff: Backoff,
}

impl RetrySpanExporter {
    pub fn new(exporter: SpanExporter) -> Self {
        Self { inner: Arc::new(Mutex::new(exporter)), backoff: Backoff::new() }
    }
}

impl SpanExporterTrait for RetrySpanExporter {
    async fn export(&self, batch: Vec<SpanData>) -> Result<(), OTelSdkError> {
        let this = self.clone();

        let mut delay = this.backoff.init_delay();
        loop {
            match this.inner.lock().await.export(batch.clone()).await {
                Ok(()) => return Ok(()),
                Err(e) => {
                    let total_delay = this.backoff.step(&mut delay);
                    tracing::warn!(%e, "failed to export spans, retrying in {total_delay:?}");
                    time::sleep(total_delay).await;
                }
            }
        }
    }

    fn shutdown(&mut self) -> Result<(), OTelSdkError> {
        block_in_place(|| self.inner.blocking_lock().shutdown())
    }

    fn force_flush(&mut self) -> Result<(), OTelSdkError> {
        let this = self.clone();
        block_in_place(|| this.inner.blocking_lock().force_flush())
    }

    fn set_resource(&mut self, resource: &Resource) {
        block_in_place(|| {
            self.inner.blocking_lock().set_resource(resource);
        });
    }
}

/// Add retry logic to a `MetricExporter`.
pub struct RetryMetricExporter {
    inner: MetricExporter,
    backoff: Backoff,
}

impl RetryMetricExporter {
    pub const fn new(exporter: MetricExporter) -> Self {
        Self { inner: exporter, backoff: Backoff::new() }
    }
}

impl PushMetricExporter for RetryMetricExporter {
    async fn export(&self, metrics: &ResourceMetrics) -> Result<(), OTelSdkError> {
        let mut delay = self.backoff.init_delay();
        loop {
            match self.inner.export(metrics).await {
                Ok(()) => return Ok(()),
                Err(err) => {
                    let total_delay = self.backoff.step(&mut delay);
                    tracing::warn!(%err, "Failed to export metrics, retrying in {total_delay:?}");
                    tokio::time::sleep(total_delay).await;
                }
            }
        }
    }

    fn force_flush(&self) -> Result<(), OTelSdkError> {
        self.inner.force_flush()
    }

    fn shutdown(&self) -> Result<(), OTelSdkError> {
        self.inner.shutdown()
    }

    fn temporality(&self) -> Temporality {
        self.inner.temporality()
    }

    fn shutdown_with_timeout(&self, _: Duration) -> Result<(), OTelSdkError> {
        self.inner.shutdown()
    }
}

/// Exponential backoff and retry strategy to use when exporting metrics and
/// traces fails.
#[derive(Debug, Clone, Copy)]
pub struct Backoff {
    init_delay: Duration,
    growth_factor: f64,
    max_delay: Duration,
    /// If this is set to 50 then the actual delay will be delay +/- a random
    /// value in range [0..delay / 2].
    jitter_percent: u64,
}

impl Backoff {
    pub const fn new() -> Self {
        Self {
            init_delay: Duration::from_millis(500),
            growth_factor: 2.0,
            max_delay: Duration::from_secs(30),
            jitter_percent: 50,
        }
    }

    const fn init_delay(&self) -> Duration {
        self.init_delay
    }

    /// Returns current delay with jitter, and updates the delay.
    #[allow(clippy::cast_possible_truncation)]
    #[allow(clippy::cast_precision_loss)]
    #[allow(clippy::cast_sign_loss)]
    fn step(&self, delay: &mut Duration) -> Duration {
        let jitter = Duration::from_millis(
            rand::rng().random_range(0..delay.as_millis() as u64 * self.jitter_percent / 100),
        );
        let total_delay = *delay + jitter;
        *delay = Duration::from_millis(((*delay).as_millis() as f64 * self.growth_factor) as u64);
        if *delay > self.max_delay {
            *delay = self.max_delay;
        }
        total_delay
    }
}
