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
/// Spawns `drive_epoch` and `sample_pool` off the runtime's engine, calls
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

/// A guest's process exit status.
///
/// The one-shot `wasi:cli` trigger drives a `wasi:cli/run` guest exactly once
/// and reports its exit code through this newtype. The generated `main`
/// converts it to a [`std::process::ExitCode`] at the process boundary.
///
/// The status rides an `Arc<OnceLock<ExitStatus>>` side channel rather than a
/// return value because [`serve`] returns `Result<()>` and discards each
/// server's value — the same way the HTTP trigger delivers its response out of
/// band (over the socket) instead of through `run`'s return type.
///
/// # Truncation
///
/// [`code`](Self::code) preserves the full `i32` a guest reports, but a process
/// exit status is only 8 bits on POSIX. The [`ExitCode`](std::process::ExitCode)
/// conversion (and [`code_u8`](Self::code_u8)) therefore keeps just the low
/// byte, matching the `wasmtime` CLI: `256` becomes `0`, `257` becomes `1`, and
/// `-1` becomes `255`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ExitStatus(i32);

impl ExitStatus {
    /// The success status (exit code `0`).
    pub const SUCCESS: Self = Self(0);

    /// The wrapped exit code, as the guest reported it (full `i32`).
    #[must_use]
    pub const fn code(self) -> i32 {
        self.0
    }

    /// The exit code truncated to the low 8 bits — the value a process actually
    /// surfaces on POSIX (and what the [`ExitCode`](std::process::ExitCode)
    /// conversion uses). See [the truncation note](Self#truncation).
    #[must_use]
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    pub const fn code_u8(self) -> u8 {
        self.0 as u8
    }
}

impl From<i32> for ExitStatus {
    fn from(code: i32) -> Self {
        Self(code)
    }
}

impl From<ExitStatus> for std::process::ExitCode {
    fn from(status: ExitStatus) -> Self {
        Self::from(status.code_u8())
    }
}

#[cfg(test)]
mod tests {
    use super::ExitStatus;

    #[test]
    fn success_is_zero() {
        assert_eq!(ExitStatus::SUCCESS.code(), 0);
        assert_eq!(ExitStatus::SUCCESS.code_u8(), 0);
    }

    #[test]
    fn from_i32_preserves_full_code() {
        // `code()` keeps the whole i32; only the byte view / `ExitCode`
        // conversion truncates.
        assert_eq!(ExitStatus::from(2).code(), 2);
        assert_eq!(ExitStatus::from(256).code(), 256);
        assert_eq!(ExitStatus::from(-1).code(), -1);
    }

    #[test]
    fn code_u8_keeps_low_byte() {
        assert_eq!(ExitStatus::from(0).code_u8(), 0);
        assert_eq!(ExitStatus::from(2).code_u8(), 2);
        assert_eq!(ExitStatus::from(255).code_u8(), 255);
        assert_eq!(ExitStatus::from(256).code_u8(), 0);
        assert_eq!(ExitStatus::from(257).code_u8(), 1);
        assert_eq!(ExitStatus::from(-1).code_u8(), 255);
        // The `ExitCode` conversion runs (its value is opaque, so only the
        // byte rule above is asserted).
        let _ = std::process::ExitCode::from(ExitStatus::from(2));
    }

    #[test]
    fn is_copy_and_eq() {
        let status = ExitStatus::from(7);
        let copied = status;
        assert_eq!(status, copied);
    }
}
