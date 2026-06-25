//! # Runtime background tasks
//!
//! Detached background tasks that drive a running runtime off its Wasmtime
//! [`Engine`]: epoch interruption (so guest deadlines fire while CPU-bound
//! guests execute) and pooling-allocator occupancy sampling (emitted as
//! `OpenTelemetry` gauges via the `tracing` metrics bridge so pool sizing can
//! be tuned from real data rather than guesswork).

use std::time::Duration;

use wasmtime::Engine;

/// Spawn a detached background task that drives epoch interruption.
///
/// Calls [`Engine::increment_epoch`] every `tick`. Together with the per-store
/// epoch deadline installed in `Runtime::build_store`, this is what lets a
/// CPU-bound guest periodically yield to the async executor so the wall-clock
/// timeout wrapped around each invocation can fire.
///
/// `tick` must be non-zero: the runtime clamps `EPOCH_TICK_MS` to a 1ms minimum
/// (see `parse_tick`), and [`tokio::time::interval`] panics on a zero period.
pub fn drive_epoch(engine: Engine, tick: Duration) {
    tokio::spawn(async move {
        let mut interval = tokio::time::interval(tick);
        loop {
            interval.tick().await;
            engine.increment_epoch();
        }
    });
}

/// Spawn a detached background task that samples pool occupancy as metrics.
///
/// Periodically reads `engine`'s pooling-allocator occupancy and emits it as
/// `OpenTelemetry` gauges (through the `tracing` -> `OpenTelemetry` metrics
/// layer configured in `omnia-otel`).
///
/// The task is a no-op and is never spawned when `interval` is zero. If the
/// engine was not configured with the pooling allocator (so there are no pool
/// metrics to report) the task stops after its first tick.
pub fn sample_pool(engine: Engine, interval: Duration) {
    if interval.is_zero() {
        return;
    }

    tokio::spawn(async move {
        let mut ticker = tokio::time::interval(interval);
        loop {
            ticker.tick().await;

            let Some(metrics) = engine.pooling_allocator_metrics() else {
                break;
            };

            tracing::info!(
                gauge.pool_core_instances = metrics.core_instances(),
                gauge.pool_component_instances = metrics.component_instances(),
                gauge.pool_memories = metrics.memories() as u64,
                gauge.pool_tables = metrics.tables() as u64,
                gauge.pool_stacks = metrics.stacks() as u64,
                gauge.pool_unused_warm_memories = u64::from(metrics.unused_warm_memories()),
                gauge.pool_unused_memory_bytes_resident =
                    metrics.unused_memory_bytes_resident() as u64,
            );
        }
    });
}
