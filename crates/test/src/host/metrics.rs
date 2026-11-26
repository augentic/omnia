//! The runtime's own metrics, recorded in memory for a test to read back.

use opentelemetry::global;
use opentelemetry_sdk::metrics::data::{
    AggregatedMetrics, MetricData, ResourceMetrics, ScopeMetrics, SumDataPoint,
};
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
        self.counted(counter, |_| true)
    }

    /// The total the runtime has counted on `counter` under the attribute
    /// `key` set to `value`, or `0` when it has counted nothing there.
    ///
    /// # Panics
    ///
    /// Panics if the provider cannot flush or the exporter cannot be read,
    /// or if `counter` names a metric that is not a `u64` counter.
    #[must_use]
    pub fn count_with(&self, counter: &str, key: &str, value: &str) -> u64 {
        self.counted(counter, |point| {
            point
                .attributes()
                .any(|attribute| attribute.key.as_str() == key && attribute.value.as_str() == value)
        })
    }

    fn counted(&self, counter: &str, keep: impl Fn(&SumDataPoint<u64>) -> bool) -> u64 {
        let Some(latest) = self.exported() else {
            return 0;
        };

        latest
            .scope_metrics()
            .flat_map(ScopeMetrics::metrics)
            .filter(|metric| metric.name() == counter)
            .map(|metric| match metric.data() {
                AggregatedMetrics::U64(MetricData::Sum(sum)) if sum.is_monotonic() => sum
                    .data_points()
                    .filter(|point| keep(point))
                    .map(SumDataPoint::value)
                    .sum::<u64>(),
                _ => panic!("`{counter}` is not a u64 counter"),
            })
            .sum()
    }

    /// The values the runtime has recorded on `histogram`, across every set
    /// of attributes, or none when it has recorded nothing there.
    ///
    /// # Panics
    ///
    /// Panics if the provider cannot flush or the exporter cannot be read,
    /// or if `histogram` names a metric that is not an `f64` histogram.
    #[must_use]
    pub fn observations(&self, histogram: &str) -> Observations {
        let Some(latest) = self.exported() else {
            return Observations::default();
        };

        latest
            .scope_metrics()
            .flat_map(ScopeMetrics::metrics)
            .filter(|metric| metric.name() == histogram)
            .flat_map(|metric| match metric.data() {
                AggregatedMetrics::F64(MetricData::Histogram(recorded)) => recorded.data_points(),
                _ => panic!("`{histogram}` is not an f64 histogram"),
            })
            .fold(Observations::default(), |total, point| Observations {
                count: total.count + point.count(),
                sum: total.sum + point.sum(),
            })
    }

    // the provider's last export, once a flush has made it current
    fn exported(&self) -> Option<ResourceMetrics> {
        self.provider.force_flush().expect("the meter provider flushes");
        self.exporter.get_finished_metrics().expect("the exporter reads back").pop()
    }
}

/// The values the runtime has recorded on one histogram, as [`Metrics::observations`] reads them.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Observations {
    /// How many values were recorded.
    pub count: u64,
    /// The sum of the values recorded.
    pub sum: f64,
}
