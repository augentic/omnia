//! # Runtime background tasks
//!
//! Detached background tasks that drive a running runtime off its Wasmtime
//! [`Engine`]: epoch interruption (so guest deadlines fire while CPU-bound
//! guests execute) and pooling-allocator occupancy sampling (emitted as
//! `OpenTelemetry` gauges via the `tracing` metrics bridge so pool sizing can
//! be tuned from real data rather than guesswork).

use std::time::Duration;

use anyhow::{Context as _, Result};
use futures::future::{BoxFuture, try_join_all};
use wasmtime::Engine;
use wasmtime_wasi::WasiView;
use wrpc_wasmtime::WrpcView;

use crate::dispatch::serve_links;
use crate::traits::Runtime;

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
/// layer configured by [`Telemetry`]).
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

/// Drive a runtime's lifecycle: start its background tasks, wire the serve side
/// of any host-mediated links, then run every trigger server to completion.
///
/// Spawns [`drive_epoch`] and [`sample_pool`] off the runtime's engine, calls
/// [`serve_links`] so a dispatched call always finds its target's wRPC server
/// (a no-op when no `link`s are declared), then awaits all `servers` together.
/// Every server shares the runtime's single [`Registry`](crate::Registry) and
/// therefore one `Engine`, so per-request instantiation draws from one pool.
///
/// This is the fixed orchestration the `runtime!` macro previously inlined; the
/// only deployment-specific input is the `servers` list.
///
/// # Errors
///
/// Returns an error if wiring the link serve side fails, or if any server
/// returns an error (the first error cancels the rest).
pub async fn serve<R: Runtime>(runtime: &R, servers: Vec<BoxFuture<'_, Result<()>>>) -> Result<()>
where
    R::StoreCtx: WasiView + WrpcView + 'static,
{
    // Drive epoch interruption so guest deadlines (and the wall-clock timeouts
    // wrapped around each invocation) fire even while a guest executes
    // CPU-bound code.
    drive_epoch(runtime.registry().engine().clone(), runtime.options().epoch_tick);

    // Periodically sample pool occupancy as metrics so pool sizing can be tuned
    // from real data.
    sample_pool(runtime.registry().engine().clone(), runtime.options().pool_metrics_interval);

    // Wire the serve side of any host-mediated links before triggers fire, so a
    // dispatched call always finds its target's wRPC server.
    serve_links(runtime).await.context("wiring host-mediated link serve side")?;

    try_join_all(servers).await?;
    Ok(())
}
