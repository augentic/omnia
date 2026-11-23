//! # Runtime configuration
//!
//! Centralised, environment-driven configuration for the runtime engine and
//! per-guest stores.
//!
//! The compile-affecting settings are `omnia-compile`'s [`CompileOptions`];
//! the runtime's [`Config`] (the `From<&RuntimeOptions>` conversion) applies
//! the copy it loaded from the environment through the one body the
//! ahead-of-time compiler goes through, [`CompileOptions::configure`], which
//! is what lets the loading engine accept a pre-compiled artifact.

use std::num::{NonZeroUsize, ParseIntError};
use std::time::Duration;

use anyhow::{Result, bail};
use omnia_compile::CompileOptions;
use wasmtime::{Config, Enabled, InstanceAllocationStrategy, PoolingAllocationConfig};

use crate::FromEnv;

/// Runtime configuration loaded from the environment.
///
/// Read once at start-up and threaded through the runtime to every store.
/// Each field maps to one environment variable (booleans use `true`/`false`),
/// which its doc names with its default. [`Default`] holds those defaults.
///
/// The fields marked compile-affecting are the [`CompileOptions`] this
/// runtime loads artifacts under ([`compile_options`](Self::compile_options))
/// and must match between pre-compiling a component and loading it. The rest
/// only affect the engine or individual stores at runtime.
#[expect(clippy::struct_excessive_bools, reason = "flat by design: one environment variable each")]
#[derive(Clone, Debug, FromEnv)]
#[env(crate = crate, validate = Self::validate)]
pub struct RuntimeOptions {
    /// Wall-clock cap on a server or server-rooted link-dispatch invocation (`GUEST_TIMEOUT_MS`, default 30s, min 1ms; a command-mode chain, including its link hops, is uncapped).
    #[env(from = "GUEST_TIMEOUT_MS", default = "30000", with = crate::env::millis)]
    pub guest_timeout: Duration,
    /// Wall-clock cap on an HTTP response body still streaming after its head is sent (`HTTP_STREAM_TIMEOUT_MS`, unset: no cap, min 1ms).
    #[env(from = "HTTP_STREAM_TIMEOUT_MS", with = crate::env::millis)]
    pub http_stream_timeout: Option<Duration>,
    /// Epoch-increment interval, also the CPU-bound guest yield granularity (`EPOCH_TICK_MS`, default 10ms, min 1ms).
    #[env(from = "EPOCH_TICK_MS", default = "10", with = parse_tick)]
    pub epoch_tick: Duration,
    /// Maximum linear-memory size a guest may grow to, in bytes (`MAX_MEMORY_BYTES`, default 256 `MiB`).
    #[env(from = "MAX_MEMORY_BYTES", default = "268435456")]
    pub max_memory_bytes: usize,
    /// Largest single file or socket read or write the host performs for a guest, in bytes (`WASI_MAX_IO_SIZE`, unset: Wasmtime default 64 `KiB`).
    #[env(from = "WASI_MAX_IO_SIZE")]
    pub wasi_max_io_size: Option<NonZeroUsize>,
    /// Virtual address space reserved per linear memory, in bytes; compile-affecting (`MEMORY_RESERVATION`, unset: Wasmtime default).
    #[env(from = "MEMORY_RESERVATION")]
    pub memory_reservation: Option<u64>,
    /// Unmapped guard region after each linear memory, in bytes; compile-affecting (`MEMORY_GUARD_SIZE`, unset: Wasmtime default).
    #[env(from = "MEMORY_GUARD_SIZE")]
    pub memory_guard_size: Option<u64>,
    /// Extra bytes reserved beyond current size to absorb growth without remapping (`MEMORY_RESERVATION_FOR_GROWTH`, unset: Wasmtime default).
    #[env(from = "MEMORY_RESERVATION_FOR_GROWTH")]
    pub memory_reservation_for_growth: Option<u64>,
    /// Zero async (fiber) stacks before reuse (`ASYNC_STACK_ZEROING`, default `false`).
    #[env(from = "ASYNC_STACK_ZEROING", default = "false")]
    pub async_stack_zeroing: bool,
    /// Capture guest backtraces and attach them to trap errors (`WASM_BACKTRACE`, default `false`).
    #[env(from = "WASM_BACKTRACE", default = "false")]
    pub wasm_backtrace: bool,
    /// Emit ELF symbol tables in compiled artifacts, for profilers and `wasmtime objdump`; compile-affecting (`DEBUG_SYMBOLS`, default `false`).
    #[env(from = "DEBUG_SYMBOLS", default = "false")]
    pub debug_symbols: bool,
    /// Record the machine-code-to-wasm-offset map that gives traps and backtraces their wasm offsets; compile-affecting (`GENERATE_ADDRESS_MAP`, default `true`).
    #[env(from = "GENERATE_ADDRESS_MAP", default = "true")]
    pub generate_address_map: bool,
    /// Per-invocation fuel budget; `0` disables metering (`MAX_FUEL`, default 0).
    #[env(from = "MAX_FUEL", default = "0")]
    pub max_fuel: u64,
    /// Maximum host-mediated guest-to-guest dispatch nesting depth (`MAX_DISPATCH_DEPTH`, default 8).
    #[env(from = "MAX_DISPATCH_DEPTH", default = "8")]
    pub max_dispatch_depth: usize,
    /// Most invocations the triggers and the command driver run at once, each holding one slot of the concurrency gate (`MAX_CONCURRENT_INVOCATIONS`, unset: `POOL_MAX_INSTANCES`, min 1).
    #[env(from = "MAX_CONCURRENT_INVOCATIONS")]
    pub max_concurrent_invocations: Option<u32>,
    /// Enable the pooling instance allocator (`POOLING`, default `true`).
    #[env(from = "POOLING", default = "true")]
    pub pooling: bool,
    /// Maximum component instances held by the pooling allocator (`POOL_MAX_INSTANCES`, default 1000).
    #[env(from = "POOL_MAX_INSTANCES", default = "1000")]
    pub pool_max_instances: u32,
    /// Linear-memory size reserved per pooled memory, in bytes (`POOL_MAX_MEMORY_BYTES`, unset: inherits `max_memory_bytes`).
    #[env(from = "POOL_MAX_MEMORY_BYTES")]
    pub pool_max_memory_bytes: Option<usize>,
    /// Bytes of each pooled linear memory kept resident on slot reuse (`POOL_MEMORY_KEEP_RESIDENT`, default 0).
    #[env(from = "POOL_MEMORY_KEEP_RESIDENT", default = "0")]
    pub pool_memory_keep_resident: usize,
    /// Bytes of each pooled table kept resident on slot reuse (`POOL_TABLE_KEEP_RESIDENT`, default 0).
    #[env(from = "POOL_TABLE_KEEP_RESIDENT", default = "0")]
    pub pool_table_keep_resident: usize,
    /// Bytes of each pooled async stack kept resident on slot reuse (`POOL_ASYNC_STACK_KEEP_RESIDENT`, default 0).
    #[env(from = "POOL_ASYNC_STACK_KEEP_RESIDENT", default = "0")]
    pub pool_async_stack_keep_resident: usize,
    /// Unused warm slots retained for fast reuse (`POOL_MAX_UNUSED_WARM_SLOTS`, default 100).
    #[env(from = "POOL_MAX_UNUSED_WARM_SLOTS", default = "100")]
    pub pool_max_unused_warm_slots: u32,
    /// Maximum core instances held by the pooling allocator (`POOL_TOTAL_CORE_INSTANCES`, unset: four per `POOL_MAX_INSTANCES`); see [`total_core_instances`](Self::total_core_instances).
    #[env(from = "POOL_TOTAL_CORE_INSTANCES")]
    pub pool_total_core_instances: Option<u32>,
    /// Maximum linear memories held by the pooling allocator (`POOL_TOTAL_MEMORIES`, unset: `POOL_MAX_INSTANCES`); see [`total_memories`](Self::total_memories).
    #[env(from = "POOL_TOTAL_MEMORIES")]
    pub pool_total_memories: Option<u32>,
    /// Maximum tables held by the pooling allocator (`POOL_TOTAL_TABLES`, unset: four per `POOL_MAX_INSTANCES`); see [`total_tables`](Self::total_tables).
    #[env(from = "POOL_TOTAL_TABLES")]
    pub pool_total_tables: Option<u32>,
    /// Maximum async stacks held by the pooling allocator (`POOL_TOTAL_STACKS`, unset: two per `POOL_MAX_INSTANCES`); see [`total_stacks`](Self::total_stacks).
    #[env(from = "POOL_TOTAL_STACKS")]
    pub pool_total_stacks: Option<u32>,
    /// Max core instances per component (`POOL_MAX_CORE_INSTANCES_PER_COMPONENT`, unset: Wasmtime default).
    #[env(from = "POOL_MAX_CORE_INSTANCES_PER_COMPONENT")]
    pub pool_max_core_instances_per_component: Option<u32>,
    /// Max linear memories per component (`POOL_MAX_MEMORIES_PER_COMPONENT`, unset: Wasmtime default).
    #[env(from = "POOL_MAX_MEMORIES_PER_COMPONENT")]
    pub pool_max_memories_per_component: Option<u32>,
    /// Max tables per component (`POOL_MAX_TABLES_PER_COMPONENT`, unset: Wasmtime default).
    #[env(from = "POOL_MAX_TABLES_PER_COMPONENT")]
    pub pool_max_tables_per_component: Option<u32>,
    /// Max linear memories per core module (`POOL_MAX_MEMORIES_PER_MODULE`, unset: Wasmtime default 1).
    #[env(from = "POOL_MAX_MEMORIES_PER_MODULE")]
    pub pool_max_memories_per_module: Option<u32>,
    /// Max tables per core module (`POOL_MAX_TABLES_PER_MODULE`, unset: Wasmtime default 1).
    #[env(from = "POOL_MAX_TABLES_PER_MODULE")]
    pub pool_max_tables_per_module: Option<u32>,
    /// Max `VMContext` size per core instance, in bytes (`POOL_MAX_CORE_INSTANCE_SIZE`, unset: Wasmtime default 1 `MiB`).
    #[env(from = "POOL_MAX_CORE_INSTANCE_SIZE")]
    pub pool_max_core_instance_size: Option<usize>,
    /// Max metadata size per component instance, in bytes (`POOL_MAX_COMPONENT_INSTANCE_SIZE`, unset: Wasmtime default 1 `MiB`).
    #[env(from = "POOL_MAX_COMPONENT_INSTANCE_SIZE")]
    pub pool_max_component_instance_size: Option<usize>,
    /// Slots batched per decommit to amortise syscalls; on Linux a whole batch flushes in one `process_madvise` (`POOL_DECOMMIT_BATCH_SIZE`, default 16).
    #[env(from = "POOL_DECOMMIT_BATCH_SIZE", default = "16")]
    pub pool_decommit_batch_size: usize,
    /// Use the Linux `PAGEMAP_SCAN` ioctl for cheaper memory reset (`POOL_PAGEMAP_SCAN`, `auto`/`yes`/`no`, default `no`).
    #[env(from = "POOL_PAGEMAP_SCAN", default = "no", with = parse_enabled)]
    pub pool_pagemap_scan: Enabled,
    /// Interval between occupancy samples of the instance pool and the concurrency gate; `0` disables (`POOL_METRICS_INTERVAL_MS`, default 5000ms).
    #[env(from = "POOL_METRICS_INTERVAL_MS", default = "5000", with = crate::env::millis)]
    pub pool_metrics_interval: Duration,
    /// Idle time, measured from the last guest invocation, after which unused warm pooled memory is released to the OS; `0` disables (`POOL_IDLE_RELEASE_MS`, default 0).
    #[env(from = "POOL_IDLE_RELEASE_MS", default = "0", with = crate::env::millis)]
    pub pool_idle_release: Duration,
    /// Honour WebAssembly branch hints during compilation; compile-affecting (`BRANCH_HINTING`, default `false`).
    #[env(from = "BRANCH_HINTING", default = "false")]
    pub branch_hinting: bool,
}

impl RuntimeOptions {
    /// The compile-affecting settings among these options, as the value an
    /// artifact this runtime loads must have been compiled under.
    #[must_use]
    pub const fn compile_options(&self) -> CompileOptions {
        CompileOptions {
            max_fuel: self.max_fuel,
            branch_hinting: self.branch_hinting,
            memory_reservation: self.memory_reservation,
            memory_guard_size: self.memory_guard_size,
            debug_symbols: self.debug_symbols,
            generate_address_map: self.generate_address_map,
        }
    }

    /// Checks that a runtime can run under these options.
    ///
    /// [`load_env`](Self::load_env), [`with_env`](Self::with_env), and
    /// [`load_from`](Self::load_from) run it on what they load. A deployment
    /// runs it on the options it is given before it builds the engine, where
    /// wasmtime's own rejection is less specific.
    ///
    /// # Errors
    ///
    /// Returns an error when:
    /// - [`guest_timeout`](Self::guest_timeout) or [`epoch_tick`](Self::epoch_tick)
    ///   is zero;
    /// - [`http_stream_timeout`](Self::http_stream_timeout) is set to zero;
    /// - [`max_concurrent_invocations`](Self::max_concurrent_invocations) is
    ///   set to zero;
    /// - with [`pooling`](Self::pooling) on,
    ///   [`max_concurrent_invocations`](Self::max_concurrent_invocations)
    ///   exceeds [`pool_max_instances`](Self::pool_max_instances);
    /// - with [`pooling`](Self::pooling) on,
    ///   [`pool_max_memories_per_module`](Self::pool_max_memories_per_module)
    ///   exceeds [`total_memories`](Self::total_memories);
    /// - with [`pooling`](Self::pooling) on,
    ///   [`pool_max_tables_per_module`](Self::pool_max_tables_per_module)
    ///   exceeds [`total_tables`](Self::total_tables).
    pub fn validate(&self) -> Result<()> {
        if self.guest_timeout.is_zero() {
            bail!(
                "GUEST_TIMEOUT_MS must be at least 1 (a zero timeout would fail every invocation)"
            );
        }
        if self.epoch_tick.is_zero() {
            bail!("EPOCH_TICK_MS must be at least 1");
        }
        if self.http_stream_timeout.is_some_and(|limit| limit.is_zero()) {
            bail!("HTTP_STREAM_TIMEOUT_MS must be at least 1; unset it for no cap");
        }
        if self.max_concurrent_invocations == Some(0) {
            bail!(
                "MAX_CONCURRENT_INVOCATIONS must be at least 1; unset it for one slot per pooled \
                 instance"
            );
        }
        if !self.pooling {
            return Ok(());
        }

        if let Some(gate) = self.max_concurrent_invocations
            && gate > self.pool_max_instances
        {
            bail!(
                "MAX_CONCURRENT_INVOCATIONS ({gate}) exceeds POOL_MAX_INSTANCES ({})",
                self.pool_max_instances
            );
        }
        if let Some(per_module) = self.pool_max_memories_per_module
            && per_module > self.total_memories()
        {
            bail!(
                "POOL_MAX_MEMORIES_PER_MODULE ({per_module}) exceeds POOL_TOTAL_MEMORIES ({})",
                self.total_memories()
            );
        }
        if let Some(per_module) = self.pool_max_tables_per_module
            && per_module > self.total_tables()
        {
            bail!(
                "POOL_MAX_TABLES_PER_MODULE ({per_module}) exceeds POOL_TOTAL_TABLES ({})",
                self.total_tables()
            );
        }

        Ok(())
    }

    /// The core instances the pool holds: `POOL_TOTAL_CORE_INSTANCES`, or
    /// four for each of `POOL_MAX_INSTANCES`.
    #[must_use]
    pub fn total_core_instances(&self) -> u32 {
        self.pool_total_core_instances
            .unwrap_or_else(|| self.pool_max_instances.saturating_mul(CORE_INSTANCES_PER_INSTANCE))
    }

    /// The linear memories the pool holds: `POOL_TOTAL_MEMORIES`, or one for
    /// each of `POOL_MAX_INSTANCES`.
    #[must_use]
    pub fn total_memories(&self) -> u32 {
        self.pool_total_memories.unwrap_or(self.pool_max_instances)
    }

    /// The tables the pool holds: `POOL_TOTAL_TABLES`, or four for each of
    /// `POOL_MAX_INSTANCES`.
    #[must_use]
    pub fn total_tables(&self) -> u32 {
        self.pool_total_tables
            .unwrap_or_else(|| self.pool_max_instances.saturating_mul(TABLES_PER_INSTANCE))
    }

    /// The async stacks the pool holds: `POOL_TOTAL_STACKS`, or two for each
    /// of `POOL_MAX_INSTANCES`.
    #[must_use]
    pub fn total_stacks(&self) -> u32 {
        self.pool_total_stacks
            .unwrap_or_else(|| self.pool_max_instances.saturating_mul(STACKS_PER_INSTANCE))
    }
}

// a typical guest takes three core instances, one memory and two tables
const CORE_INSTANCES_PER_INSTANCE: u32 = 4;
const TABLES_PER_INSTANCE: u32 = 4;

// a guest blocked in a call keeps its stack while wasmtime completes another
// of its calls on a second
const STACKS_PER_INSTANCE: u32 = 2;

// compile-affecting settings through `configure`, then the runtime-only ones
impl From<&RuntimeOptions> for Config {
    fn from(options: &RuntimeOptions) -> Self {
        let mut config = Self::new();
        options.compile_options().configure(&mut config);

        // runtime-only engine settings, before the pooling early-return
        config.async_stack_zeroing(options.async_stack_zeroing);
        if let Some(bytes) = options.memory_reservation_for_growth {
            config.memory_reservation_for_growth(bytes);
        }

        // backtraces off via the max-frames api; `true` leaves the default on
        if !options.wasm_backtrace {
            config.wasm_backtrace_max_frames(None);
        }

        if !options.pooling {
            return config;
        }

        // SECURITY: the pooling allocator + CoW is exactly the configuration
        // historical Wasmtime advisories target, so the workspace `wasmtime`
        // pin tracks upstream releases promptly; this is where a lagging pin
        // matters most.
        let mut pool = PoolingAllocationConfig::new();

        pool.total_component_instances(options.pool_max_instances)
            .total_core_instances(options.total_core_instances())
            .total_memories(options.total_memories())
            .total_tables(options.total_tables())
            .total_stacks(options.total_stacks())
            .max_memory_size(options.pool_max_memory_bytes.unwrap_or(options.max_memory_bytes))
            // keep-resident skips decommit and zeroing on slot reuse
            .linear_memory_keep_resident(options.pool_memory_keep_resident)
            .table_keep_resident(options.pool_table_keep_resident)
            .async_stack_keep_resident(options.pool_async_stack_keep_resident)
            .max_unused_warm_slots(options.pool_max_unused_warm_slots)
            // batching amortises the per-teardown decommit syscalls
            .decommit_batch_size(options.pool_decommit_batch_size)
            // linux-only fast memory reset; `Auto` falls back where unsupported
            .pagemap_scan(options.pool_pagemap_scan);

        // structural limits only when set, so unset keeps the wasmtime default
        if let Some(count) = options.pool_max_core_instances_per_component {
            pool.max_core_instances_per_component(count);
        }
        if let Some(count) = options.pool_max_memories_per_component {
            pool.max_memories_per_component(count);
        }
        if let Some(count) = options.pool_max_tables_per_component {
            pool.max_tables_per_component(count);
        }
        if let Some(count) = options.pool_max_memories_per_module {
            pool.max_memories_per_module(count);
        }
        if let Some(count) = options.pool_max_tables_per_module {
            pool.max_tables_per_module(count);
        }
        if let Some(size) = options.pool_max_core_instance_size {
            pool.max_core_instance_size(size);
        }
        if let Some(size) = options.pool_max_component_instance_size {
            pool.max_component_instance_size(size);
        }

        config.allocation_strategy(InstanceAllocationStrategy::Pooling(pool));
        config
    }
}

// clamped so the ticker interval can never be zero
fn parse_tick(value: &str) -> Result<Duration, ParseIntError> {
    crate::env::millis(value).map(|tick| tick.max(Duration::from_millis(1)))
}

fn parse_enabled(value: &str) -> Result<Enabled, &'static str> {
    match value.trim().to_ascii_lowercase().as_str() {
        "auto" => Ok(Enabled::Auto),
        "yes" | "true" | "on" | "1" => Ok(Enabled::Yes),
        "no" | "false" | "off" | "0" => Ok(Enabled::No),
        _ => Err("expected one of `auto`/`yes`/`no`"),
    }
}

// The totals and the refusals are pure over the options. The pool a runtime
// builds from them is exercised by every suite that boots one.
#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::RuntimeOptions;

    fn options(instances: u32) -> RuntimeOptions {
        RuntimeOptions {
            pool_max_instances: instances,
            ..RuntimeOptions::default()
        }
    }

    #[test]
    fn unset_totals() {
        let options = options(10);

        assert_eq!(options.total_core_instances(), 40);
        assert_eq!(options.total_memories(), 10);
        assert_eq!(options.total_tables(), 40);
        assert_eq!(options.total_stacks(), 20);
    }

    #[test]
    fn set_totals() {
        let options = RuntimeOptions {
            pool_total_core_instances: Some(7),
            pool_total_memories: Some(8),
            pool_total_tables: Some(9),
            pool_total_stacks: Some(11),
            ..options(10)
        };

        assert_eq!(options.total_core_instances(), 7);
        assert_eq!(options.total_memories(), 8);
        assert_eq!(options.total_tables(), 9);
        assert_eq!(options.total_stacks(), 11);
    }

    #[test]
    fn saturating_totals() {
        let options = options(u32::MAX);

        assert_eq!(options.total_core_instances(), u32::MAX);
        assert_eq!(options.total_tables(), u32::MAX);
        assert_eq!(options.total_stacks(), u32::MAX);
    }

    #[test]
    fn per_module_over_derived_total() {
        let options = RuntimeOptions {
            pool_max_memories_per_module: Some(3),
            ..options(2)
        };

        let error =
            options.validate().expect_err("three memories per module exceed a total of two");
        assert!(
            error
                .to_string()
                .contains("POOL_MAX_MEMORIES_PER_MODULE (3) exceeds POOL_TOTAL_MEMORIES (2)"),
            "{error}"
        );
    }

    #[test]
    fn per_module_tables_over_total() {
        let options = RuntimeOptions {
            pool_total_tables: Some(8),
            pool_max_tables_per_module: Some(16),
            ..RuntimeOptions::default()
        };

        let error = options.validate().expect_err("tables per module exceed the total");
        assert!(
            error
                .to_string()
                .contains("POOL_MAX_TABLES_PER_MODULE (16) exceeds POOL_TOTAL_TABLES (8)"),
            "{error}"
        );
    }

    #[test]
    fn pooling_off() {
        let options = RuntimeOptions {
            pooling: false,
            pool_total_memories: Some(8),
            pool_max_memories_per_module: Some(16),
            ..RuntimeOptions::default()
        };

        options.validate().expect("pool bounds are ignored with pooling off");
    }

    #[test]
    fn zero_stream_timeout() {
        let options = RuntimeOptions {
            pooling: false,
            http_stream_timeout: Some(Duration::ZERO),
            ..RuntimeOptions::default()
        };

        let error = options.validate().expect_err("a zero stream cap is refused");
        assert!(error.to_string().contains("HTTP_STREAM_TIMEOUT_MS"), "{error}");
    }

    #[test]
    fn zero_gate() {
        let options = RuntimeOptions {
            pooling: false,
            max_concurrent_invocations: Some(0),
            ..RuntimeOptions::default()
        };

        let error = options.validate().expect_err("a zero gate is refused");
        assert!(error.to_string().contains("MAX_CONCURRENT_INVOCATIONS"), "{error}");
    }

    // with pooling off the gate is the only bound, so any width stands
    #[test]
    fn wide_gate() {
        let options = RuntimeOptions {
            max_concurrent_invocations: Some(11),
            ..options(10)
        };

        let error = options.validate().expect_err("a gate wider than the pool is refused");
        assert!(
            error
                .to_string()
                .contains("MAX_CONCURRENT_INVOCATIONS (11) exceeds POOL_MAX_INSTANCES (10)"),
            "{error}"
        );

        let unpooled = RuntimeOptions {
            pooling: false,
            ..options
        };
        unpooled.validate().expect("a gate wider than an unused pool stands");
    }

    #[test]
    fn zero_tick() {
        let options = RuntimeOptions {
            pooling: false,
            epoch_tick: Duration::ZERO,
            ..RuntimeOptions::default()
        };

        let error = options.validate().expect_err("a zero tick is refused");
        assert!(error.to_string().contains("EPOCH_TICK_MS"), "{error}");
    }
}
