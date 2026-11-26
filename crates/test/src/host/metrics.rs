//! The runtime's own metrics, recorded in memory for a test to read back.

use opentelemetry::global;
use opentelemetry_sdk::metrics::data::{AggregatedMetrics, MetricData, ScopeMetrics, SumDataPoint};
use opentelemetry_sdk::metrics::{InMemoryMetricExporter, SdkMeterProvider};

/// The runtime's own metrics, recorded in memory for a test to read back.
///
/// The runtime binds its metrics once per process, when the first
/// [`Deployment`](super::Deployment) boots, to the global meter provider as
/// it then stands. Call [`install`](Self::install) once per process, before
/// that boot. Under `cargo nextest`, each test runs in a process of its own.
#[derive(Debug)]
pub struct Metrics {
    provider: SdkMeterProvider,
    exporter: InMemoryMetricExporter,
}

impl Metrics {
    /// Publishes a meter provider that records in memory, beside a `tracing`
    /// subscriber that writes to the test's captured output.
    ///
    /// The runtime installs its own subscriber and providers only when no
    /// subscriber is set, so a later boot keeps these two.
    ///
    /// # Panics
    ///
    /// Panics if the process already has a global `tracing` subscriber, as a
    /// second install or an earlier boot leaves.
    #[must_use]
    pub fn install() -> Self {
        tracing_subscriber::fmt().with_test_writer().try_init().expect(
            "no tracing subscriber is set yet: install `Metrics` once per process, before the \
             first deployment boots",
        );

        let exporter = InMemoryMetricExporter::default();
        let provider = SdkMeterProvider::builder().with_periodic_exporter(exporter.clone()).build();
        global::set_meter_provider(provider.clone());
        Self { provider, exporter }
    }

    /// The total the runtime has counted on `counter`, across every set of
    /// attributes, or `0` when it has counted nothing there.
    ///
    /// # Panics
    ///
    /// Panics if the provider cannot flush or the exporter cannot be read,
    /// or if `counter` names a metric that is not a `u64` counter.
    #[must_use]
    pub fn count(&self, counter: &str) -> u64 {
        self.provider.force_flush().expect("the meter provider flushes");
        let finished = self.exporter.get_finished_metrics().expect("the exporter reads back");
        let Some(latest) = finished.last() else {
            return 0;
        };

        latest
            .scope_metrics()
            .flat_map(ScopeMetrics::metrics)
            .filter(|metric| metric.name() == counter)
            .map(|metric| match metric.data() {
                AggregatedMetrics::U64(MetricData::Sum(sum)) if sum.is_monotonic() => {
                    sum.data_points().map(SumDataPoint::value).sum::<u64>()
                }
                _ => panic!("`{counter}` is not a u64 counter"),
            })
            .sum()
    }
}
