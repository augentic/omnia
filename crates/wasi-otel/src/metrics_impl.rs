use omnia_core::HostView;
use opentelemetry_proto::tonic::collector::metrics::v1::ExportMetricsServiceRequest;
use opentelemetry_proto::tonic::metrics::v1::exemplar::Value as ExemplarValue;
use opentelemetry_proto::tonic::metrics::v1::exponential_histogram_data_point::Buckets;
use opentelemetry_proto::tonic::metrics::v1::metric::Data as MetricData;
use opentelemetry_proto::tonic::metrics::v1::number_data_point::Value as NumberValue;
use opentelemetry_proto::tonic::metrics::v1::{
    AggregationTemporality, Exemplar, ExponentialHistogram, ExponentialHistogramDataPoint, Gauge,
    Histogram, HistogramDataPoint, Metric, NumberDataPoint, ResourceMetrics, ScopeMetrics, Sum,
};
use wasmtime::component::Accessor;

use crate::generated::omnia::otel::metrics::{self as wasi, Host, HostWithStore};
use crate::{Error, OtelClient, Result, WasiOtel, types_impl};

impl<T> HostWithStore<T> for WasiOtel {
    async fn export(
        accessor: &Accessor<T, Self>, scope_metrics: Vec<wasi::ScopeMetrics>,
    ) -> Result<()> {
        if scope_metrics.iter().all(|group| group.metrics.is_empty()) {
            return Ok(());
        }
        let scope_metrics =
            scope_metrics.into_iter().map(ScopeMetrics::try_from).collect::<Result<Vec<_>>>()?;
        let Some((resource, schema_url)) = types_impl::resource() else {
            return Ok(());
        };

        let request = ExportMetricsServiceRequest {
            resource_metrics: vec![ResourceMetrics {
                resource: Some(resource),
                scope_metrics,
                schema_url,
            }],
        };
        accessor.with(|mut store| store.get().client.export_metrics(request)).await?;
        Ok(())
    }
}

impl Host for HostView<'_, dyn OtelClient> {}

impl TryFrom<wasi::ScopeMetrics> for ScopeMetrics {
    type Error = Error;

    fn try_from(group: wasi::ScopeMetrics) -> Result<Self> {
        Ok(Self {
            scope: group.scope.map(TryInto::try_into).transpose()?,
            metrics: try_map(group.metrics)?,
            schema_url: group.schema_url.unwrap_or_default(),
        })
    }
}

impl TryFrom<wasi::Metric> for Metric {
    type Error = Error;

    fn try_from(metric: wasi::Metric) -> Result<Self> {
        let data = MetricData::try_from(metric.data)
            .map_err(|err| types_impl::within(err, &format!("metric `{}`", metric.name)))?;
        Ok(Self {
            name: metric.name,
            description: metric.description.unwrap_or_default(),
            unit: metric.unit.unwrap_or_default(),
            metadata: Vec::new(),
            data: Some(data),
        })
    }
}

impl TryFrom<wasi::MetricData> for MetricData {
    type Error = Error;

    fn try_from(data: wasi::MetricData) -> Result<Self> {
        Ok(match data {
            wasi::MetricData::Gauge(gauge) => Self::Gauge(Gauge {
                data_points: try_map(gauge.data_points)?,
            }),
            wasi::MetricData::Sum(sum) => Self::Sum(Sum {
                data_points: try_map(sum.data_points)?,
                aggregation_temporality: temporality(sum.aggregation_temporality),
                is_monotonic: sum.is_monotonic,
            }),
            wasi::MetricData::Histogram(histogram) => Self::Histogram(Histogram {
                data_points: try_map(histogram.data_points)?,
                aggregation_temporality: temporality(histogram.aggregation_temporality),
            }),
            wasi::MetricData::ExponentialHistogram(histogram) => {
                Self::ExponentialHistogram(ExponentialHistogram {
                    data_points: try_map(histogram.data_points)?,
                    aggregation_temporality: temporality(histogram.aggregation_temporality),
                })
            }
        })
    }
}

fn try_map<T, U: TryFrom<T, Error = Error>>(items: Vec<T>) -> Result<Vec<U>> {
    items.into_iter().map(U::try_from).collect()
}

fn temporality(temporality: wasi::AggregationTemporality) -> i32 {
    match temporality {
        wasi::AggregationTemporality::Delta => AggregationTemporality::Delta,
        wasi::AggregationTemporality::Cumulative => AggregationTemporality::Cumulative,
    }
    .into()
}

// The proto `DataPointFlags` default (`FLAG_NONE` / "do not use").
const DATA_POINT_FLAGS_DO_NOT_USE: u32 = 0;

impl TryFrom<wasi::NumberDataPoint> for NumberDataPoint {
    type Error = Error;

    fn try_from(point: wasi::NumberDataPoint) -> Result<Self> {
        Ok(Self {
            attributes: types_impl::attributes(point.attributes)?,
            start_time_unix_nano: point.start_time_unix_nano.unwrap_or_default(),
            time_unix_nano: point.time_unix_nano,
            exemplars: try_map(point.exemplars)?,
            flags: DATA_POINT_FLAGS_DO_NOT_USE,
            value: Some(point.value.into()),
        })
    }
}

impl TryFrom<wasi::HistogramDataPoint> for HistogramDataPoint {
    type Error = Error;

    fn try_from(point: wasi::HistogramDataPoint) -> Result<Self> {
        Ok(Self {
            attributes: types_impl::attributes(point.attributes)?,
            start_time_unix_nano: point.start_time_unix_nano.unwrap_or_default(),
            time_unix_nano: point.time_unix_nano,
            count: point.count,
            sum: point.sum,
            bucket_counts: point.bucket_counts,
            explicit_bounds: point.explicit_bounds,
            exemplars: try_map(point.exemplars)?,
            flags: DATA_POINT_FLAGS_DO_NOT_USE,
            min: point.min,
            max: point.max,
        })
    }
}

impl TryFrom<wasi::ExponentialHistogramDataPoint> for ExponentialHistogramDataPoint {
    type Error = Error;

    fn try_from(point: wasi::ExponentialHistogramDataPoint) -> Result<Self> {
        Ok(Self {
            attributes: types_impl::attributes(point.attributes)?,
            start_time_unix_nano: point.start_time_unix_nano.unwrap_or_default(),
            time_unix_nano: point.time_unix_nano,
            count: point.count,
            sum: point.sum,
            scale: point.scale,
            zero_count: point.zero_count,
            positive: Some(point.positive.into()),
            negative: Some(point.negative.into()),
            flags: DATA_POINT_FLAGS_DO_NOT_USE,
            exemplars: try_map(point.exemplars)?,
            min: point.min,
            max: point.max,
            zero_threshold: point.zero_threshold,
        })
    }
}

impl From<wasi::Buckets> for Buckets {
    fn from(buckets: wasi::Buckets) -> Self {
        Self {
            offset: buckets.offset,
            bucket_counts: buckets.bucket_counts,
        }
    }
}

impl TryFrom<wasi::Exemplar> for Exemplar {
    type Error = Error;

    fn try_from(exemplar: wasi::Exemplar) -> Result<Self> {
        let span_id =
            exemplar.span_id.map(|id| types_impl::span_id(&id, "exemplar span id")).transpose()?;
        let trace_id = exemplar
            .trace_id
            .map(|id| types_impl::trace_id(&id, "exemplar trace id"))
            .transpose()?;
        Ok(Self {
            filtered_attributes: types_impl::attributes(exemplar.filtered_attributes)?,
            time_unix_nano: exemplar.time_unix_nano,
            span_id: span_id.unwrap_or_default(),
            trace_id: trace_id.unwrap_or_default(),
            value: Some(exemplar.value.into()),
        })
    }
}

impl From<wasi::Number> for NumberValue {
    fn from(number: wasi::Number) -> Self {
        match number {
            wasi::Number::AsDouble(v) => Self::AsDouble(v),
            wasi::Number::AsInt(v) => Self::AsInt(v),
        }
    }
}

impl From<wasi::Number> for ExemplarValue {
    fn from(number: wasi::Number) -> Self {
        match number {
            wasi::Number::AsDouble(v) => Self::AsDouble(v),
            wasi::Number::AsInt(v) => Self::AsInt(v),
        }
    }
}
