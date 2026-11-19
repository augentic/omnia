//! # Metrics
//!
//! The `opentelemetry` metrics API implemented over the `omnia:otel` records:
//! synchronous instruments aggregate into a shared registry that [`export`]
//! collects into one `scope-metrics` record per scope for the host.
//! Observable (callback) instruments keep the API's no-op defaults.

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

use crate::otel::aggregate::{
    self, Aggregate, GaugeData, HistogramData, LastValue, Measurement, Sum, SumData,
};
use crate::otel::convert::unix_nanos;
use crate::otel::metrics as wasi;

/// Provides meters whose synchronous instruments record into a shared
/// registry.
#[derive(Clone, Debug, Default)]
pub struct MeterProvider {
    registry: Arc<Registry>,
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
                    scope: Some((&scope.scope).into()),
                    metrics,
                    schema_url: scope.scope.schema_url().map(Into::into),
                })
            })
            .collect()
    }
}

impl opentelemetry::metrics::MeterProvider for MeterProvider {
    fn meter_with_scope(&self, scope: InstrumentationScope) -> Meter {
        Meter::new(Arc::new(Instruments {
            scope,
            registry: Arc::clone(&self.registry),
        }))
    }
}

/// Export all recorded metrics to the host; a collection with no data
/// points is skipped rather than exported empty.
pub async fn export(provider: &MeterProvider) {
    let scope_metrics = provider.collect(SystemTime::now());
    if scope_metrics.is_empty() {
        return;
    }

    if let Err(e) = wasi::export(scope_metrics).await {
        tracing::error!("failed to export metrics: {e}");
    }
}

// every instrument built so far, grouped by instrumentation scope
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
    // A later build of the same instrument shares the series and keeps the
    // first description and unit, as in the SDK.
    fn series<A>(
        &mut self, kind: Kind, name: Cow<'static, str>, description: Option<Cow<'static, str>>,
        unit: Option<Cow<'static, str>>, aggregate: impl FnOnce(SystemTime) -> A,
    ) -> Arc<Series<A>>
    where
        A: Aggregate,
        A::Data: Into<wasi::MetricData>,
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
                description,
                unit,
                state: Mutex::new(aggregate(SystemTime::now())),
            });
            self.instruments.push(Arc::clone(&series) as Arc<dyn Instrument>);
            series
        })
    }
}

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
        A::Data: Into<wasi::MetricData>,
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

    fn histogram<T: Measurement>(
        &self, kind: Kind, builder: HistogramBuilder<'_, Histogram<T>>,
    ) -> Arc<dyn SyncInstrument<T> + Send + Sync> {
        let bounds = builder.boundaries.unwrap_or_else(|| aggregate::DEFAULT_BOUNDARIES.to_vec());
        if !aggregate::valid_boundaries(&bounds) {
            // as in the sdk, a misconfigured instrument records nothing
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

// an instrument's identity within its scope
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

trait Instrument: Any + fmt::Debug + Send + Sync {
    fn kind(&self) -> Kind;
    fn name(&self) -> &str;
    fn collect(&self, now: SystemTime) -> Option<wasi::Metric>;
    fn into_any(self: Arc<Self>) -> Arc<dyn Any + Send + Sync>;
}

#[derive(Debug)]
struct Series<A> {
    kind: Kind,
    name: Cow<'static, str>,
    description: Option<Cow<'static, str>>,
    unit: Option<Cow<'static, str>>,
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
    A::Data: Into<wasi::MetricData>,
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
            description: self.description.as_deref().map(Into::into),
            unit: self.unit.as_deref().map(Into::into),
            data: data.into(),
        })
    }

    fn into_any(self: Arc<Self>) -> Arc<dyn Any + Send + Sync> {
        self
    }
}

// what a misconfigured builder yields
#[derive(Debug)]
struct Noop;

impl<T> SyncInstrument<T> for Noop {
    fn measure(&self, _: T, _: &[KeyValue]) {}
}

// One impl per source type, so an `f64` keeps its fraction. OTLP's integer
// is signed, so a `u64` past `i64::MAX` saturates rather than wrapping.
impl From<u64> for wasi::Number {
    fn from(value: u64) -> Self {
        Self::AsInt(i64::try_from(value).unwrap_or(i64::MAX))
    }
}

impl From<i64> for wasi::Number {
    fn from(value: i64) -> Self {
        Self::AsInt(value)
    }
}

impl From<f64> for wasi::Number {
    fn from(value: f64) -> Self {
        Self::AsDouble(value)
    }
}

impl<T: Measurement + Into<wasi::Number>> From<SumData<T>> for wasi::MetricData {
    fn from(sum: SumData<T>) -> Self {
        let (start, time) = (unix_nanos(sum.start_time), unix_nanos(sum.time));
        Self::Sum(wasi::Sum {
            data_points: sum
                .points
                .into_iter()
                .map(|(attributes, value)| number_point(attributes, value, start, time))
                .collect(),
            aggregation_temporality: if sum.delta {
                wasi::AggregationTemporality::Delta
            } else {
                wasi::AggregationTemporality::Cumulative
            },
            is_monotonic: sum.monotonic,
        })
    }
}

impl<T: Measurement + Into<wasi::Number>> From<GaugeData<T>> for wasi::MetricData {
    fn from(gauge: GaugeData<T>) -> Self {
        let (start, time) = (unix_nanos(gauge.start_time), unix_nanos(gauge.time));
        Self::Gauge(wasi::Gauge {
            data_points: gauge
                .points
                .into_iter()
                .map(|(attributes, value)| number_point(attributes, value, start, time))
                .collect(),
        })
    }
}

fn number_point(
    attributes: Vec<KeyValue>, value: impl Into<wasi::Number>, start: u64, time: u64,
) -> wasi::NumberDataPoint {
    wasi::NumberDataPoint {
        attributes: convert(attributes),
        start_time_unix_nano: Some(start),
        time_unix_nano: time,
        value: value.into(),
        exemplars: Vec::new(),
    }
}

fn convert(attributes: Vec<KeyValue>) -> Vec<wasi::KeyValue> {
    attributes.into_iter().map(Into::into).collect()
}

impl<T: Measurement> From<HistogramData<T>> for wasi::MetricData {
    fn from(histogram: HistogramData<T>) -> Self {
        let HistogramData {
            points,
            bounds,
            start_time,
            time,
        } = histogram;
        let (start, time) = (unix_nanos(start_time), unix_nanos(time));
        Self::Histogram(wasi::Histogram {
            data_points: points
                .into_iter()
                .map(|point| wasi::HistogramDataPoint {
                    attributes: convert(point.attributes),
                    start_time_unix_nano: Some(start),
                    time_unix_nano: time,
                    count: point.count,
                    sum: Some(point.sum.as_f64()),
                    bucket_counts: point.bucket_counts,
                    explicit_bounds: bounds.clone(),
                    exemplars: Vec::new(),
                    min: Some(point.min.as_f64()),
                    max: Some(point.max.as_f64()),
                })
                .collect(),
            aggregation_temporality: wasi::AggregationTemporality::Delta,
        })
    }
}
