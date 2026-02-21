//! # Metrics

use std::sync::{Arc, Weak};
use std::time::Duration;

use num_traits::ToPrimitive;
use opentelemetry::global;
use opentelemetry_sdk::Resource;
use opentelemetry_sdk::error::OTelSdkResult;
use opentelemetry_sdk::metrics::data::{
    AggregatedMetrics, Exemplar, ExponentialBucket, ExponentialHistogram,
    ExponentialHistogramDataPoint, Gauge, GaugeDataPoint, Histogram, HistogramDataPoint, Metric,
    MetricData, ResourceMetrics, ScopeMetrics, Sum, SumDataPoint,
};
use opentelemetry_sdk::metrics::reader::MetricReader;
use opentelemetry_sdk::metrics::{
    InstrumentKind, ManualReader, Pipeline, SdkMeterProvider, Temporality,
};

use crate::guest::generated::wasi::otel::metrics as wasi;

pub fn init(resource: Resource) -> SdkMeterProvider {
    let reader = Reader::new();
    let provider = SdkMeterProvider::builder().with_resource(resource).with_reader(reader).build();
    global::set_meter_provider(provider.clone());
    provider
}

#[derive(Debug, Clone)]
struct Reader(Arc<ManualReader>);

impl Reader {
    #[must_use]
    fn new() -> Self {
        Self(Arc::new(ManualReader::default()))
    }
}

impl MetricReader for Reader {
    fn register_pipeline(&self, pipeline: Weak<Pipeline>) {
        self.0.register_pipeline(pipeline);
    }

    fn collect(&self, rm: &mut ResourceMetrics) -> OTelSdkResult {
        self.0.collect(rm)
    }

    fn force_flush(&self) -> OTelSdkResult {
        self.0.force_flush()
    }

    fn temporality(&self, kind: InstrumentKind) -> Temporality {
        self.0.temporality(kind)
    }

    fn shutdown_with_timeout(&self, _: Duration) -> OTelSdkResult {
        let mut rm = ResourceMetrics::default();
        self.0.collect(&mut rm)?;

        wit_bindgen::spawn(async move {
            let metrics: wasi::ResourceMetrics = rm.into();
            if let Err(e) = wasi::export(metrics).await {
                tracing::error!("failed to export metrics: {e}");
            }
        });

        Ok(())
    }
}

impl From<ResourceMetrics> for wasi::ResourceMetrics {
    fn from(rm: ResourceMetrics) -> Self {
        Self {
            resource: rm.resource().into(),
            scope_metrics: rm.scope_metrics().map(Into::into).collect(),
        }
    }
}

impl From<&Resource> for wasi::Resource {
    fn from(resource: &Resource) -> Self {
        Self {
            attributes: resource.iter().map(Into::into).collect(),
            schema_url: resource.schema_url().map(ToString::to_string),
        }
    }
}

impl From<&ScopeMetrics> for wasi::ScopeMetrics {
    fn from(scope_metrics: &ScopeMetrics) -> Self {
        Self {
            scope: scope_metrics.scope().into(),
            metrics: scope_metrics.metrics().map(Into::into).collect(),
        }
    }
}

impl From<&Metric> for wasi::Metric {
    fn from(metric: &Metric) -> Self {
        Self {
            name: metric.name().to_string(),
            description: metric.description().to_string(),
            unit: metric.unit().to_string(),
            data: metric.data().into(),
        }
    }
}

impl From<&AggregatedMetrics> for wasi::AggregatedMetrics {
    fn from(am: &AggregatedMetrics) -> Self {
        match am {
            AggregatedMetrics::F64(v) => Self::F64(v.into()),
            AggregatedMetrics::I64(v) => Self::S64(v.into()),
            AggregatedMetrics::U64(v) => Self::U64(v.into()),
        }
    }
}

impl<T: ToPrimitive + Copy> From<&MetricData<T>> for wasi::MetricData {
    fn from(md: &MetricData<T>) -> Self {
        match md {
            MetricData::Gauge(v) => Self::Gauge(v.into()),
            MetricData::Sum(v) => Self::Sum(v.into()),
            MetricData::Histogram(v) => Self::Histogram(v.into()),
            MetricData::ExponentialHistogram(v) => Self::ExponentialHistogram(v.into()),
        }
    }
}

impl<T: ToPrimitive + Copy> From<&Gauge<T>> for wasi::Gauge {
    fn from(gauge: &Gauge<T>) -> Self {
        Self {
            data_points: gauge.data_points().map(Into::into).collect(),
            start_time: gauge.start_time().map(Into::into),
            time: gauge.time().into(),
        }
    }
}

impl<T: ToPrimitive + Copy> From<&GaugeDataPoint<T>> for wasi::GaugeDataPoint {
    fn from(data_point: &GaugeDataPoint<T>) -> Self {
        Self {
            attributes: data_point.attributes().map(Into::into).collect(),
            value: data_point.value().into(),
            exemplars: data_point.exemplars().map(Into::into).collect(),
        }
    }
}

impl<T: ToPrimitive + Copy> From<&Exemplar<T>> for wasi::Exemplar {
    fn from(exemplar: &Exemplar<T>) -> Self {
        Self {
            filtered_attributes: exemplar.filtered_attributes().map(Into::into).collect(),
            time: exemplar.time().into(),
            value: exemplar.value.into(),
            span_id: String::from_utf8_lossy(exemplar.span_id()).into(),
            trace_id: String::from_utf8_lossy(exemplar.trace_id()).into(),
        }
    }
}

impl<T: ToPrimitive + Copy> From<&Sum<T>> for wasi::Sum {
    fn from(sum: &Sum<T>) -> Self {
        Self {
            data_points: sum.data_points().map(Into::into).collect(),
            start_time: sum.start_time().into(),
            time: sum.time().into(),
            temporality: sum.temporality().into(),
            is_monotonic: sum.is_monotonic(),
        }
    }
}

impl<T: ToPrimitive + Copy> From<&SumDataPoint<T>> for wasi::SumDataPoint {
    fn from(data_point: &SumDataPoint<T>) -> Self {
        Self {
            attributes: data_point.attributes().map(Into::into).collect(),
            value: data_point.value().into(),
            exemplars: data_point.exemplars().map(Into::into).collect(),
        }
    }
}

impl<T: ToPrimitive + Copy> From<&Histogram<T>> for wasi::Histogram {
    fn from(histogram: &Histogram<T>) -> Self {
        Self {
            data_points: histogram.data_points().map(Into::into).collect(),
            start_time: histogram.start_time().into(),
            time: histogram.time().into(),
            temporality: histogram.temporality().into(),
        }
    }
}

impl<T: ToPrimitive + Copy> From<&HistogramDataPoint<T>> for wasi::HistogramDataPoint {
    fn from(data_point: &HistogramDataPoint<T>) -> Self {
        Self {
            attributes: data_point.attributes().map(Into::into).collect(),
            count: data_point.count(),
            bounds: data_point.bounds().collect(),
            bucket_counts: data_point.bucket_counts().collect(),
            min: data_point.min().map(Into::into),
            max: data_point.max().map(Into::into),
            sum: data_point.sum().into(),
            exemplars: data_point.exemplars().map(Into::into).collect(),
        }
    }
}

impl<T: ToPrimitive + Copy> From<&ExponentialHistogram<T>> for wasi::ExponentialHistogram {
    fn from(histogram: &ExponentialHistogram<T>) -> Self {
        Self {
            data_points: histogram.data_points().map(Into::into).collect(),
            start_time: histogram.start_time().into(),
            time: histogram.time().into(),
            temporality: histogram.temporality().into(),
        }
    }
}

impl<T: ToPrimitive + Copy> From<&ExponentialHistogramDataPoint<T>>
    for wasi::ExponentialHistogramDataPoint
{
    fn from(data_point: &ExponentialHistogramDataPoint<T>) -> Self {
        Self {
            attributes: data_point.attributes().map(Into::into).collect(),
            scale: data_point.scale(),
            zero_count: data_point.zero_count(),
            positive_bucket: data_point.positive_bucket().into(),
            negative_bucket: data_point.negative_bucket().into(),
            zero_threshold: data_point.zero_threshold(),
            min: data_point.min().map(Into::into),
            max: data_point.max().map(Into::into),
            sum: data_point.sum().into(),
            count: data_point.count() as u64,
            exemplars: data_point.exemplars().map(Into::into).collect(),
        }
    }
}

impl<T: ToPrimitive> From<T> for wasi::DataValue {
    fn from(value: T) -> Self {
        value.to_u64().map_or_else(
            || {
                value
                    .to_i64()
                    .map_or_else(|| Self::F64(value.to_f64().unwrap_or_default()), Self::S64)
            },
            Self::U64,
        )
    }
}

impl From<&ExponentialBucket> for wasi::ExponentialBucket {
    fn from(bucket: &ExponentialBucket) -> Self {
        Self {
            offset: bucket.offset(),
            counts: bucket.counts().collect(),
        }
    }
}

impl From<Temporality> for wasi::Temporality {
    fn from(temporality: Temporality) -> Self {
        match temporality {
            Temporality::Delta => Self::Delta,
            _ => Self::Cumulative,
        }
    }
}
