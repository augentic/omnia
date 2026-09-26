//! # Aggregate
//!
//! The accumulators behind the guest's synchronous instruments: what each
//! records per attribute set and what one collection yields. `metrics` wraps
//! them in the `opentelemetry` instrument handles and converts each
//! collection into the `omnia:otel` records.

use std::collections::{BTreeMap, HashMap};
use std::fmt;
use std::time::SystemTime;

use opentelemetry::KeyValue;

/// The bucket boundaries a histogram uses when its builder sets none.
pub const DEFAULT_BOUNDARIES: [f64; 15] = [
    0.0, 5.0, 10.0, 25.0, 50.0, 75.0, 100.0, 250.0, 500.0, 750.0, 1000.0, 2500.0, 5000.0, 7500.0,
    10000.0,
];

/// A value type an instrument records.
pub trait Measurement: Copy + Default + PartialOrd + fmt::Debug + Send + Sync + 'static {
    /// Adds without panicking: integers saturate.
    fn add(self, other: Self) -> Self;

    /// The value on the histogram boundary scale.
    fn as_f64(self) -> f64;
}

impl Measurement for u64 {
    fn add(self, other: Self) -> Self {
        self.saturating_add(other)
    }

    #[expect(clippy::cast_precision_loss)]
    fn as_f64(self) -> f64 {
        self as f64
    }
}

impl Measurement for i64 {
    fn add(self, other: Self) -> Self {
        self.saturating_add(other)
    }

    #[expect(clippy::cast_precision_loss)]
    fn as_f64(self) -> f64 {
        self as f64
    }
}

impl Measurement for f64 {
    fn add(self, other: Self) -> Self {
        self + other
    }

    fn as_f64(self) -> f64 {
        self
    }
}

/// An attribute set as a time-series key: sorted by key, the last value
/// winning for a duplicated key.
pub type Attributes = Vec<KeyValue>;

fn attribute_set(attributes: &[KeyValue]) -> Attributes {
    attributes
        .iter()
        .map(|kv| (kv.key.clone(), kv.value.clone()))
        .collect::<BTreeMap<_, _>>()
        .into_iter()
        .map(|(key, value)| KeyValue::new(key, value))
        .collect()
}

/// The accumulator behind one instrument.
pub trait Aggregate: fmt::Debug + Send + Sync + 'static {
    /// The measurement type the instrument records.
    type Value: Measurement;
    /// One collection's worth of data points.
    type Data;

    /// Folds a measurement into the series for `attributes`.
    fn record(&mut self, value: Self::Value, attributes: &[KeyValue]);

    /// Produces the data points recorded so far, or `None` when there are
    /// none; a delta aggregation starts over.
    fn collect(&mut self, now: SystemTime) -> Option<Self::Data>;
}

/// Running totals per attribute set: a counter or an up-down counter.
#[derive(Debug)]
pub struct Sum<T> {
    totals: HashMap<Attributes, T>,
    start_time: SystemTime,
    monotonic: bool,
    delta: bool,
}

/// One collection of a [`Sum`].
#[derive(Debug)]
pub struct SumData<T> {
    /// The total per attribute set.
    pub points: Vec<(Attributes, T)>,
    /// When the reported interval began.
    pub start_time: SystemTime,
    /// When the collection happened.
    pub time: SystemTime,
    /// Whether the totals only ever increase.
    pub monotonic: bool,
    /// Whether the totals cover only the interval since the last collection.
    pub delta: bool,
}

impl<T> Sum<T> {
    /// A sum that only increases when `monotonic`, and that starts over at
    /// each collection when `delta`.
    pub fn new(monotonic: bool, delta: bool, now: SystemTime) -> Self {
        Self {
            totals: HashMap::new(),
            start_time: now,
            monotonic,
            delta,
        }
    }
}

impl<T: Measurement> Aggregate for Sum<T> {
    type Data = SumData<T>;
    type Value = T;

    fn record(&mut self, value: T, attributes: &[KeyValue]) {
        // a negative increment on a monotonic counter is dropped, as in the sdk
        if self.monotonic && value < T::default() {
            return;
        }
        let total = self.totals.entry(attribute_set(attributes)).or_default();
        *total = total.add(value);
    }

    fn collect(&mut self, now: SystemTime) -> Option<SumData<T>> {
        if self.totals.is_empty() {
            return None;
        }
        let (points, start_time) = if self.delta {
            let points = std::mem::take(&mut self.totals).into_iter().collect();
            (points, std::mem::replace(&mut self.start_time, now))
        } else {
            let points = self.totals.iter().map(|(attrs, total)| (attrs.clone(), *total)).collect();
            (points, self.start_time)
        };
        Some(SumData {
            points,
            start_time,
            time: now,
            monotonic: self.monotonic,
            delta: self.delta,
        })
    }
}

/// The last value recorded per attribute set: a gauge.
#[derive(Debug)]
pub struct LastValue<T> {
    values: HashMap<Attributes, T>,
    start_time: SystemTime,
}

/// One collection of a [`LastValue`].
#[derive(Debug)]
pub struct GaugeData<T> {
    /// The last value per attribute set recorded since the previous collection.
    pub points: Vec<(Attributes, T)>,
    /// When the reported interval began.
    pub start_time: SystemTime,
    /// When the collection happened.
    pub time: SystemTime,
}

impl<T> LastValue<T> {
    /// A gauge whose first interval begins at `now`.
    pub fn new(now: SystemTime) -> Self {
        Self {
            values: HashMap::new(),
            start_time: now,
        }
    }
}

impl<T: Measurement> Aggregate for LastValue<T> {
    type Data = GaugeData<T>;
    type Value = T;

    fn record(&mut self, value: T, attributes: &[KeyValue]) {
        self.values.insert(attribute_set(attributes), value);
    }

    // A gauge reports only the series recorded since the last collection, as
    // the SDK's delta gauge does.
    fn collect(&mut self, now: SystemTime) -> Option<GaugeData<T>> {
        if self.values.is_empty() {
            return None;
        }
        Some(GaugeData {
            points: std::mem::take(&mut self.values).into_iter().collect(),
            start_time: std::mem::replace(&mut self.start_time, now),
            time: now,
        })
    }
}

/// Explicit-bucket distributions per attribute set.
#[derive(Debug)]
pub struct Histogram<T> {
    bounds: Vec<f64>,
    buckets: HashMap<Attributes, Buckets<T>>,
    start_time: SystemTime,
}

#[derive(Debug)]
struct Buckets<T> {
    counts: Vec<u64>,
    count: u64,
    sum: T,
    min: T,
    max: T,
}

/// One collection of a [`Histogram`].
#[derive(Debug)]
pub struct HistogramData<T> {
    /// The distribution per attribute set.
    pub points: Vec<HistogramPoint<T>>,
    /// The upper bounds of every bucket but the open-ended last one.
    pub bounds: Vec<f64>,
    /// When the reported interval began.
    pub start_time: SystemTime,
    /// When the collection happened.
    pub time: SystemTime,
}

/// The distribution recorded for one attribute set.
#[derive(Debug)]
pub struct HistogramPoint<T> {
    /// The attribute set.
    pub attributes: Attributes,
    /// How many values were recorded.
    pub count: u64,
    /// How many values fell into each bucket; one more than the bounds.
    pub bucket_counts: Vec<u64>,
    /// The sum of the recorded values.
    pub sum: T,
    /// The smallest recorded value.
    pub min: T,
    /// The largest recorded value.
    pub max: T,
}

impl<T> Histogram<T> {
    /// A histogram over `bounds`, which must satisfy [`valid_boundaries`].
    pub fn new(bounds: Vec<f64>, now: SystemTime) -> Self {
        Self {
            bounds,
            buckets: HashMap::new(),
            start_time: now,
        }
    }
}

impl<T: Measurement> Aggregate for Histogram<T> {
    type Data = HistogramData<T>;
    type Value = T;

    fn record(&mut self, value: T, attributes: &[KeyValue]) {
        // bucket `i` holds `(bounds[i - 1], bounds[i]]`; the last is open-ended
        let index = self.bounds.partition_point(|bound| *bound < value.as_f64());
        let buckets = self.buckets.entry(attribute_set(attributes)).or_insert_with(|| Buckets {
            counts: vec![0; self.bounds.len() + 1],
            count: 0,
            sum: T::default(),
            min: value,
            max: value,
        });
        buckets.counts[index] += 1;
        buckets.count += 1;
        buckets.sum = buckets.sum.add(value);
        if value < buckets.min {
            buckets.min = value;
        }
        if value > buckets.max {
            buckets.max = value;
        }
    }

    fn collect(&mut self, now: SystemTime) -> Option<HistogramData<T>> {
        if self.buckets.is_empty() {
            return None;
        }
        let points = std::mem::take(&mut self.buckets)
            .into_iter()
            .map(|(attributes, buckets)| HistogramPoint {
                attributes,
                count: buckets.count,
                bucket_counts: buckets.counts,
                sum: buckets.sum,
                min: buckets.min,
                max: buckets.max,
            })
            .collect();
        Some(HistogramData {
            points,
            bounds: self.bounds.clone(),
            start_time: std::mem::replace(&mut self.start_time, now),
            time: now,
        })
    }
}

/// Whether histogram `bounds` are finite and strictly increasing.
pub fn valid_boundaries(bounds: &[f64]) -> bool {
    bounds.iter().all(|bound| bound.is_finite()) && bounds.is_sorted_by(|a, b| a < b)
}
