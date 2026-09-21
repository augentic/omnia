//! # Metrics
//!
//! The `opentelemetry` metrics API implemented over the `omnia:otel` records:
//! synchronous instruments aggregate into a shared registry that [`export`]
//! collects into one `resource-metrics` record for the host. Observable
//! (callback) instruments keep the API's no-op defaults.

use std::any::Any;
use std::borrow::Cow;
use std::fmt;
use std::sync::{Arc, Mutex, PoisonError};
use std::time::SystemTime;

use opentelemetry::metrics::{
    Counter, Gauge, Histogram, HistogramBuilder, InstrumentBuilder, InstrumentProvider, Meter,
    SyncInstrument, UpDownCounter,
};
use opentelemetry::{InstrumentationScope, KeyValue};

use crate::guest::aggregate::{
    self, Aggregate, GaugeData, HistogramData, LastValue, Measurement, Sum, SumData,
};
use crate::guest::generated::omnia::otel::metrics as wasi;

/// Provides meters whose synchronous instruments record into a shared
/// registry.
#[derive(Clone, Debug, Default)]
pub struct MeterProvider {
    registry: Arc<Registry>,
}

impl opentelemetry::metrics::MeterProvider for MeterProvider {
    fn meter_with_scope(&self, scope: InstrumentationScope) -> Meter {
        Meter::new(Arc::new(Instruments {
            scope,
            registry: Arc::clone(&self.registry),
        }))
    }
}

impl MeterProvider {
    // A scope without data points is left out, as is an instrument.
    fn collect(&self, now: SystemTime) -> Vec<wasi::ScopeMetrics> {
        let Ok(scopes) = self.registry.scopes.lock() else {
            return Vec::new();
        };
        scopes
            .iter()
            .filter_map(|scope| {
                let metrics: Vec<_> = scope
                    .instruments
                    .iter()
                    .filter_map(|instrument| instrument.collect(now))
                    .collect();
                (!metrics.is_empty()).then(|| wasi::ScopeMetrics {
                    scope: (&scope.scope).into(),
                    metrics,
                })
            })
            .collect()
    }
}

/// Export all recorded metrics to the host; a collection with no data
/// points is skipped rather than exported empty.
pub async fn export(provider: &MeterProvider, resource: &wasi::Resource) {
    let scope_metrics = provider.collect(SystemTime::now());
    if scope_metrics.is_empty() {
        return;
    }

    let metrics = wasi::ResourceMetrics {
        resource: resource.clone(),
        scope_metrics,
    };
    if let Err(e) = wasi::export(metrics).await {
        tracing::error!("failed to export metrics: {e}");
    }
}

/// Every instrument built so far, grouped by instrumentation scope.
#[derive(Debug, Default)]
struct Registry {
    scopes: Mutex<Vec<Scope>>,
}

#[derive(Debug)]
struct Scope {
    scope: InstrumentationScope,
    instruments: Vec<Arc<dyn Instrument>>,
}

impl Scope {
    /// The series behind (`kind`, `name`), created by `aggregate` on first
    /// use: a later build of the same instrument shares it and keeps the
    /// first description and unit, as in the SDK.
    fn series<A>(
        &mut self, kind: Kind, name: Cow<'static, str>, description: Option<Cow<'static, str>>,
        unit: Option<Cow<'static, str>>, aggregate: impl FnOnce(SystemTime) -> A,
    ) -> Arc<Series<A>>
    where
        A: Aggregate,
        A::Data: Into<wasi::AggregatedMetrics>,
    {
        let existing = self
            .instruments
            .iter()
            .find(|instrument| instrument.kind() == kind && instrument.name() == name.as_ref())
            .and_then(|instrument| Arc::clone(instrument).into_any().downcast::<Series<A>>().ok());
        existing.unwrap_or_else(|| {
            let series = Arc::new(Series {
                kind,
                name,
                description: description.unwrap_or_default(),
                unit: unit.unwrap_or_default(),
                state: Mutex::new(aggregate(SystemTime::now())),
            });
            self.instruments.push(Arc::clone(&series) as Arc<dyn Instrument>);
            series
        })
    }
}

/// The instrument builders of one meter.
#[derive(Debug)]
struct Instruments {
    scope: InstrumentationScope,
    registry: Arc<Registry>,
}

impl Instruments {
    fn series<A>(
        &self, kind: Kind, name: Cow<'static, str>, description: Option<Cow<'static, str>>,
        unit: Option<Cow<'static, str>>, aggregate: impl FnOnce(SystemTime) -> A,
    ) -> Arc<Series<A>>
    where
        A: Aggregate,
        A::Data: Into<wasi::AggregatedMetrics>,
    {
        let mut scopes = self.registry.scopes.lock().unwrap_or_else(PoisonError::into_inner);
        let index =
            scopes.iter().position(|scope| scope.scope == self.scope).unwrap_or_else(|| {
                scopes.push(Scope {
                    scope: self.scope.clone(),
                    instruments: Vec::new(),
                });
                scopes.len() - 1
            });
        scopes[index].series(kind, name, description, unit, aggregate)
    }

    fn histogram<T: Number>(
        &self, kind: Kind, builder: HistogramBuilder<'_, Histogram<T>>,
    ) -> Arc<dyn SyncInstrument<T> + Send + Sync> {
        let bounds = builder.boundaries.unwrap_or_else(|| aggregate::DEFAULT_BOUNDARIES.to_vec());
        if !aggregate::valid_boundaries(&bounds) {
            // As in the SDK, a misconfigured instrument records nothing
            // rather than failing the guest.
            tracing::error!(
                name = %builder.name,
                "histogram boundaries must be finite and strictly increasing; recording nothing"
            );
            return Arc::new(Noop);
        }
        self.series(kind, builder.name, builder.description, builder.unit, |now| {
            aggregate::Histogram::new(bounds, now)
        })
    }
}

impl InstrumentProvider for Instruments {
    fn u64_counter(&self, builder: InstrumentBuilder<'_, Counter<u64>>) -> Counter<u64> {
        Counter::new(self.series(
            Kind::CounterU64,
            builder.name,
            builder.description,
            builder.unit,
            |now| Sum::new(true, true, now),
        ))
    }

    fn f64_counter(&self, builder: InstrumentBuilder<'_, Counter<f64>>) -> Counter<f64> {
        Counter::new(self.series(
            Kind::CounterF64,
            builder.name,
            builder.description,
            builder.unit,
            |now| Sum::new(true, true, now),
        ))
    }

    // Up-down counters stay cumulative under the delta preference, as the
    // SDK prescribes.
    fn i64_up_down_counter(
        &self, builder: InstrumentBuilder<'_, UpDownCounter<i64>>,
    ) -> UpDownCounter<i64> {
        UpDownCounter::new(self.series(
            Kind::UpDownCounterI64,
            builder.name,
            builder.description,
            builder.unit,
            |now| Sum::new(false, false, now),
        ))
    }

    fn f64_up_down_counter(
        &self, builder: InstrumentBuilder<'_, UpDownCounter<f64>>,
    ) -> UpDownCounter<f64> {
        UpDownCounter::new(self.series(
            Kind::UpDownCounterF64,
            builder.name,
            builder.description,
            builder.unit,
            |now| Sum::new(false, false, now),
        ))
    }

    fn u64_gauge(&self, builder: InstrumentBuilder<'_, Gauge<u64>>) -> Gauge<u64> {
        Gauge::new(self.series(
            Kind::GaugeU64,
            builder.name,
            builder.description,
            builder.unit,
            LastValue::new,
        ))
    }

    fn f64_gauge(&self, builder: InstrumentBuilder<'_, Gauge<f64>>) -> Gauge<f64> {
        Gauge::new(self.series(
            Kind::GaugeF64,
            builder.name,
            builder.description,
            builder.unit,
            LastValue::new,
        ))
    }

    fn i64_gauge(&self, builder: InstrumentBuilder<'_, Gauge<i64>>) -> Gauge<i64> {
        Gauge::new(self.series(
            Kind::GaugeI64,
            builder.name,
            builder.description,
            builder.unit,
            LastValue::new,
        ))
    }

    fn f64_histogram(&self, builder: HistogramBuilder<'_, Histogram<f64>>) -> Histogram<f64> {
        Histogram::new(self.histogram(Kind::HistogramF64, builder))
    }

    fn u64_histogram(&self, builder: HistogramBuilder<'_, Histogram<u64>>) -> Histogram<u64> {
        Histogram::new(self.histogram(Kind::HistogramU64, builder))
    }
}

/// An instrument's identity within its scope: what it measures and in
/// which value type.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Kind {
    CounterU64,
    CounterF64,
    UpDownCounterI64,
    UpDownCounterF64,
    GaugeU64,
    GaugeI64,
    GaugeF64,
    HistogramU64,
    HistogramF64,
}

/// One built instrument: its identity, for a later builder to reuse, and
/// the collection of the series it aggregates.
trait Instrument: Any + fmt::Debug + Send + Sync {
    fn kind(&self) -> Kind;
    fn name(&self) -> &str;
    fn collect(&self, now: SystemTime) -> Option<wasi::Metric>;
    fn into_any(self: Arc<Self>) -> Arc<dyn Any + Send + Sync>;
}

/// An instrument's descriptor and accumulator.
#[derive(Debug)]
struct Series<A> {
    kind: Kind,
    name: Cow<'static, str>,
    description: Cow<'static, str>,
    unit: Cow<'static, str>,
    state: Mutex<A>,
}

impl<A: Aggregate> SyncInstrument<A::Value> for Series<A> {
    fn measure(&self, measurement: A::Value, attributes: &[KeyValue]) {
        if let Ok(mut state) = self.state.lock() {
            state.record(measurement, attributes);
        }
    }
}

impl<A> Instrument for Series<A>
where
    A: Aggregate,
    A::Data: Into<wasi::AggregatedMetrics>,
{
    fn kind(&self) -> Kind {
        self.kind
    }

    fn name(&self) -> &str {
        &self.name
    }

    fn collect(&self, now: SystemTime) -> Option<wasi::Metric> {
        let data = self.state.lock().ok()?.collect(now)?;
        Some(wasi::Metric {
            name: self.name.to_string(),
            description: self.description.to_string(),
            unit: self.unit.to_string(),
            data: data.into(),
        })
    }

    fn into_any(self: Arc<Self>) -> Arc<dyn Any + Send + Sync> {
        self
    }
}

/// An instrument that records nothing: what a misconfigured builder yields.
#[derive(Debug)]
struct Noop;

impl<T> SyncInstrument<T> for Noop {
    fn measure(&self, _: T, _: &[KeyValue]) {}
}

/// A value type with its `aggregated-metrics` arm.
trait Number: Measurement + Into<wasi::DataValue> {
    fn aggregated(data: wasi::MetricData) -> wasi::AggregatedMetrics;
}

impl Number for u64 {
    fn aggregated(data: wasi::MetricData) -> wasi::AggregatedMetrics {
        wasi::AggregatedMetrics::U64(data)
    }
}

impl Number for i64 {
    fn aggregated(data: wasi::MetricData) -> wasi::AggregatedMetrics {
        wasi::AggregatedMetrics::S64(data)
    }
}

impl Number for f64 {
    fn aggregated(data: wasi::MetricData) -> wasi::AggregatedMetrics {
        wasi::AggregatedMetrics::F64(data)
    }
}

// One impl per source type so every value crosses the boundary as itself —
// a lossy "narrowest fit" conversion would truncate fractional f64 values.
impl From<u64> for wasi::DataValue {
    fn from(value: u64) -> Self {
        Self::U64(value)
    }
}

impl From<i64> for wasi::DataValue {
    fn from(value: i64) -> Self {
        Self::S64(value)
    }
}

impl From<f64> for wasi::DataValue {
    fn from(value: f64) -> Self {
        Self::F64(value)
    }
}

impl<T: Number> From<SumData<T>> for wasi::AggregatedMetrics {
    fn from(sum: SumData<T>) -> Self {
        T::aggregated(wasi::MetricData::Sum(wasi::Sum {
            data_points: sum
                .points
                .into_iter()
                .map(|(attributes, value)| wasi::SumDataPoint {
                    attributes: convert(attributes),
                    value: value.into(),
                    exemplars: Vec::new(),
                })
                .collect(),
            start_time: sum.start_time.into(),
            time: sum.time.into(),
            temporality: if sum.delta {
                wasi::Temporality::Delta
            } else {
                wasi::Temporality::Cumulative
            },
            is_monotonic: sum.monotonic,
        }))
    }
}

impl<T: Number> From<GaugeData<T>> for wasi::AggregatedMetrics {
    fn from(gauge: GaugeData<T>) -> Self {
        T::aggregated(wasi::MetricData::Gauge(wasi::Gauge {
            data_points: gauge
                .points
                .into_iter()
                .map(|(attributes, value)| wasi::GaugeDataPoint {
                    attributes: convert(attributes),
                    value: value.into(),
                    exemplars: Vec::new(),
                })
                .collect(),
            start_time: Some(gauge.start_time.into()),
            time: gauge.time.into(),
        }))
    }
}

impl<T: Number> From<HistogramData<T>> for wasi::AggregatedMetrics {
    fn from(histogram: HistogramData<T>) -> Self {
        let HistogramData {
            points,
            bounds,
            start_time,
            time,
        } = histogram;
        T::aggregated(wasi::MetricData::Histogram(wasi::Histogram {
            data_points: points
                .into_iter()
                .map(|point| wasi::HistogramDataPoint {
                    attributes: convert(point.attributes),
                    count: point.count,
                    bounds: bounds.clone(),
                    bucket_counts: point.bucket_counts,
                    min: Some(point.min.into()),
                    max: Some(point.max.into()),
                    sum: point.sum.into(),
                    exemplars: Vec::new(),
                })
                .collect(),
            start_time: start_time.into(),
            time: time.into(),
            temporality: wasi::Temporality::Delta,
        }))
    }
}

fn convert(attributes: Vec<KeyValue>) -> Vec<wasi::KeyValue> {
    attributes.into_iter().map(Into::into).collect()
}
