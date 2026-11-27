//! The runtime's own metrics, and the resource guest telemetry carries, under
//! a subscriber the process installed first.

use omnia_core::{Telemetry, telemetry};
use opentelemetry::{Key, KeyValue, Value, global};
use opentelemetry_sdk::metrics::data::{AggregatedMetrics, Metric, MetricData, ScopeMetrics};
use opentelemetry_sdk::metrics::{InMemoryMetricExporter, SdkMeterProvider};
use tracing_subscriber::util::SubscriberInitExt;
use wasmtime::{
    Config, Engine, InstanceAllocationStrategy, PoolingAllocationConfig, PoolingAllocatorMetrics,
};

#[test]
fn installed_subscriber() {
    tracing_subscriber::registry().init();
    telemetry::count_message("early");

    let exporter = InMemoryMetricExporter::default();
    let provider = SdkMeterProvider::builder().with_periodic_exporter(exporter.clone()).build();
    global::set_meter_provider(provider.clone());
    Telemetry::new("installed-subscriber").build().expect("telemetry builds");

    let resource = telemetry::resource().expect("the resource publishes");
    assert_eq!(
        resource.get(&Key::from_static_str("service.name")),
        Some(Value::from("installed-subscriber"))
    );

    telemetry::record_pool(&idle_pool());
    telemetry::count_message("orders");
    telemetry::count_event("chat");
    telemetry::count_error("orders", Some("orders.created"));
    telemetry::count_error("chat", None);
    provider.force_flush().expect("the provider flushes");

    let finished = exporter.get_finished_metrics().expect("the exporter reads back");
    let latest = finished.last().expect("a recorded metric reached the provider");

    let scopes: Vec<_> = latest
        .scope_metrics()
        .map(|metrics| (metrics.scope().name(), metrics.scope().version()))
        .collect();
    assert_eq!(scopes, [("omnia", Some(env!("CARGO_PKG_VERSION")))]);

    let mut recorded: Vec<_> =
        latest.scope_metrics().flat_map(ScopeMetrics::metrics).flat_map(points).collect();
    recorded.sort();
    assert_eq!(
        recorded,
        [
            "counter event_counter{service=chat} 1",
            "counter message_counter{service=orders} 1",
            "counter processing_errors{service=chat} 1",
            "counter processing_errors{service=orders,topic=orders.created} 1",
            "gauge pool_component_instances{} 0",
            "gauge pool_core_instances{} 0",
            "gauge pool_memories{} 0",
            "gauge pool_stacks{} 0",
            "gauge pool_tables{} 0",
        ]
    );
}

fn idle_pool() -> PoolingAllocatorMetrics {
    let mut pool = PoolingAllocationConfig::new();
    pool.total_component_instances(1)
        .total_core_instances(1)
        .total_memories(1)
        .total_tables(1)
        .total_stacks(1);
    let mut config = Config::new();
    config.allocation_strategy(InstanceAllocationStrategy::Pooling(pool));
    let engine = Engine::new(&config).expect("pooling engine");
    engine.pooling_allocator_metrics().expect("the engine pools")
}

// each data point as `kind name{attributes} value`, attributes sorted
fn points(metric: &Metric) -> Vec<String> {
    let name = metric.name();
    match metric.data() {
        AggregatedMetrics::U64(MetricData::Gauge(gauge)) => gauge
            .data_points()
            .map(|point| line("gauge", name, point.attributes(), point.value()))
            .collect(),
        AggregatedMetrics::U64(MetricData::Sum(sum)) if sum.is_monotonic() => sum
            .data_points()
            .map(|point| line("counter", name, point.attributes(), point.value()))
            .collect(),
        _ => panic!("`{name}` is neither a u64 gauge nor a u64 counter"),
    }
}

fn line<'a>(
    kind: &str, name: &str, attributes: impl Iterator<Item = &'a KeyValue>, value: u64,
) -> String {
    let mut attributes: Vec<_> =
        attributes.map(|attribute| format!("{}={}", attribute.key, attribute.value)).collect();
    attributes.sort();
    format!("{kind} {name}{{{}}} {value}", attributes.join(","))
}
