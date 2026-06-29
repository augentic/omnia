//! # Runtime configuration
//!
//! Centralised, environment-driven configuration for the runtime engine and
//! per-guest stores.
//!
//! Building the compile-time [`Config`] in one place (the
//! `From<&RuntimeOptions>` conversion) guarantees that the engine used to
//! pre-compile a component ([`crate::compile`]) and the engine used to load it
//! ([`crate::create`]) agree on every code-affecting setting. This parity is
//! required for [`wasmtime::component::Component::deserialize_file`] to accept a
//! pre-compiled artifact.

// `derive(FromEnv)` generates undocumented `from_env`/`requirements` associated
// functions; `RuntimeOptions` is re-exported from the crate root so they would
// otherwise trip `missing_docs`.
#![allow(missing_docs)]

use std::time::Duration;

use anyhow::{Result, bail};
use fromenv::{FromEnv, ParseResult};
use wasmtime::{Config, Enabled, InstanceAllocationStrategy, PoolingAllocationConfig};

/// Runtime configuration loaded from the environment.
///
/// Values are read once at start-up via `RuntimeOptions::from_env().finalize()`
/// and threaded through the generated runtime to every store. Each field maps to
/// an `*` environment variable (booleans use `true`/`false`); call
/// [`RuntimeOptions::requirements`] to print the full list with defaults.
///
/// # Compile-time vs runtime settings
///
/// `max_fuel`, `branch_hinting`, `memory_reservation`, and `memory_guard_size`
/// influence generated code (the latter two via bounds-check elision), so they
/// are applied by the [`Config`] conversion and must be identical when a
/// component is pre-compiled and when it is later loaded. Copy-on-write heap
/// initialisation is likewise pinned there. The remaining values only affect
/// the engine or individual stores at runtime.
// A flat, env-driven configuration record; grouping the independent boolean
// toggles into enums would obscure their one-to-one mapping to environment
// variables.
#[allow(clippy::struct_excessive_bools)]
#[derive(Clone, Debug, FromEnv)]
pub struct RuntimeOptions {
    /// Wall-clock cap applied to a single guest invocation
    /// (`GUEST_TIMEOUT_MS`, default 30s).
    #[env(from = "GUEST_TIMEOUT_MS", default = "30000", with = parse_millis)]
    pub guest_timeout: Duration,
    /// Interval between [`wasmtime::Engine::increment_epoch`] ticks, and the
    /// granularity at which CPU-bound guests yield to the async executor
    /// (`EPOCH_TICK_MS`, default 10ms, clamped to a 1ms minimum).
    #[env(from = "EPOCH_TICK_MS", default = "10", with = parse_tick)]
    pub epoch_tick: Duration,
    /// Maximum linear-memory size, in bytes, a guest may grow to
    /// (`MAX_MEMORY_BYTES`, default 256 `MiB`).
    #[env(from = "MAX_MEMORY_BYTES", default = "268435456")]
    pub max_memory_bytes: usize,
    /// Bytes of virtual address space reserved up-front for each linear memory
    /// (`MEMORY_RESERVATION`). Compile-affecting: a large reservation (e.g. 4
    /// `GiB` for 32-bit guests) lets Wasmtime elide bounds checks, so this must
    /// match between `omnia compile` and `omnia run`. Unset leaves the Wasmtime
    /// default.
    #[env(from = "MEMORY_RESERVATION")]
    pub memory_reservation: Option<u64>,
    /// Bytes of unmapped guard region placed after each linear memory
    /// (`MEMORY_GUARD_SIZE`). Compile-affecting: the guard size lets Wasmtime
    /// elide and deduplicate bounds checks, so it must match between `omnia
    /// compile` and `omnia run`. Unset leaves the Wasmtime default.
    #[env(from = "MEMORY_GUARD_SIZE")]
    pub memory_guard_size: Option<u64>,
    /// Extra bytes eagerly reserved beyond a linear memory's current size to
    /// absorb growth without remapping (`MEMORY_RESERVATION_FOR_GROWTH`).
    /// Runtime-only; unset leaves the Wasmtime default.
    #[env(from = "MEMORY_RESERVATION_FOR_GROWTH")]
    pub memory_reservation_for_growth: Option<u64>,
    /// Whether to zero async (fiber) stacks before reuse
    /// (`ASYNC_STACK_ZEROING`, default `false`). Off by default for
    /// performance; enable as defense-in-depth for untrusted guests, accepting
    /// the per-instantiation cost. Runtime-only.
    #[env(from = "ASYNC_STACK_ZEROING", default = "false")]
    pub async_stack_zeroing: bool,
    /// Whether guest WebAssembly backtraces are captured and attached to trap
    /// errors (`WASM_BACKTRACE`, default `false`). Off by default to skip
    /// per-trap capture overhead; enable for richer error/`OpenTelemetry`
    /// diagnostics. Runtime-only: does not affect the compiled artifact.
    #[env(from = "WASM_BACKTRACE", default = "false")]
    pub wasm_backtrace: bool,
    /// Per-invocation fuel budget; `0` disables fuel metering
    /// (`MAX_FUEL`, default disabled).
    #[env(from = "MAX_FUEL", default = "0")]
    pub max_fuel: u64,
    /// Maximum host-mediated dynamic-linking dispatch depth — how deep a chain
    /// of guest-to-guest calls (A->B->C) may nest before the floor refuses
    /// further dispatch, bounding runaway recursion (`MAX_DISPATCH_DEPTH`,
    /// default 8). Runtime-only.
    #[env(from = "MAX_DISPATCH_DEPTH", default = "8")]
    pub max_dispatch_depth: usize,
    /// Whether the pooling instance allocator is enabled
    /// (`POOLING`, default `true`).
    #[env(from = "POOLING", default = "true")]
    pub pooling: bool,
    /// Maximum number of instances held by the pooling allocator
    /// (`POOL_MAX_INSTANCES`, default 1000).
    #[env(from = "POOL_MAX_INSTANCES", default = "1000")]
    pub pool_max_instances: u32,
    /// Maximum linear-memory size, in bytes, reserved per pooled memory
    /// (`POOL_MAX_MEMORY_BYTES`). When unset it inherits
    /// `max_memory_bytes`.
    #[env(from = "POOL_MAX_MEMORY_BYTES")]
    pub pool_max_memory_bytes: Option<usize>,
    /// Bytes of each pooled linear memory kept resident on slot reuse; a
    /// non-zero value skips the decommit/zeroing the default (`0`) forces
    /// (`POOL_MEMORY_KEEP_RESIDENT`, default 0).
    #[env(from = "POOL_MEMORY_KEEP_RESIDENT", default = "0")]
    pub pool_memory_keep_resident: usize,
    /// Bytes of each pooled table kept resident on slot reuse
    /// (`POOL_TABLE_KEEP_RESIDENT`, default 0).
    #[env(from = "POOL_TABLE_KEEP_RESIDENT", default = "0")]
    pub pool_table_keep_resident: usize,
    /// Bytes of each pooled async stack kept resident on slot reuse
    /// (`POOL_ASYNC_STACK_KEEP_RESIDENT`, default 0).
    #[env(from = "POOL_ASYNC_STACK_KEEP_RESIDENT", default = "0")]
    pub pool_async_stack_keep_resident: usize,
    /// Maximum number of unused warm slots the pooling allocator retains for
    /// fast reuse (`POOL_MAX_UNUSED_WARM_SLOTS`, default 100, matching the
    /// Wasmtime default).
    #[env(from = "POOL_MAX_UNUSED_WARM_SLOTS", default = "100")]
    pub pool_max_unused_warm_slots: u32,
    /// Maximum number of core instances held by the pooling allocator
    /// (`POOL_TOTAL_CORE_INSTANCES`, default 1000). Kept independent of the
    /// component-instance total (`POOL_MAX_INSTANCES`) so a guest whose single
    /// component embeds several core instances cannot exhaust this pool early.
    #[env(from = "POOL_TOTAL_CORE_INSTANCES", default = "1000")]
    pub pool_total_core_instances: u32,
    /// Maximum number of linear memories held by the pooling allocator
    /// (`POOL_TOTAL_MEMORIES`, default 1000). Independent of the instance
    /// total; raise this for guests that use more than one memory.
    #[env(from = "POOL_TOTAL_MEMORIES", default = "1000")]
    pub pool_total_memories: u32,
    /// Maximum number of tables held by the pooling allocator
    /// (`POOL_TOTAL_TABLES`, default 1000). Independent of the instance total;
    /// raise this for guests that use more than one table.
    #[env(from = "POOL_TOTAL_TABLES", default = "1000")]
    pub pool_total_tables: u32,
    /// Maximum number of async stacks held by the pooling allocator
    /// (`POOL_TOTAL_STACKS`, default 1000). Independent of the instance total.
    #[env(from = "POOL_TOTAL_STACKS", default = "1000")]
    pub pool_total_stacks: u32,
    /// Maximum number of garbage-collected heaps held by the pooling allocator
    /// (`POOL_TOTAL_GC_HEAPS`). Unset leaves the Wasmtime default and only takes
    /// effect when built with the opt-in `gc` feature; setting it without that
    /// feature is rejected at start-up. Only relevant to guests using the
    /// component-model GC / reference types, which current guests do not.
    #[env(from = "POOL_TOTAL_GC_HEAPS")]
    pub pool_total_gc_heaps: Option<u32>,
    /// Upper bound on the number of core instances a single component may
    /// contain (`POOL_MAX_CORE_INSTANCES_PER_COMPONENT`). Unset leaves the
    /// Wasmtime default (unlimited).
    #[env(from = "POOL_MAX_CORE_INSTANCES_PER_COMPONENT")]
    pub pool_max_core_instances_per_component: Option<u32>,
    /// Upper bound on the number of linear memories a single component may
    /// contain (`POOL_MAX_MEMORIES_PER_COMPONENT`). Unset leaves the Wasmtime
    /// default (unlimited).
    #[env(from = "POOL_MAX_MEMORIES_PER_COMPONENT")]
    pub pool_max_memories_per_component: Option<u32>,
    /// Upper bound on the number of tables a single component may contain
    /// (`POOL_MAX_TABLES_PER_COMPONENT`). Unset leaves the Wasmtime default
    /// (unlimited).
    #[env(from = "POOL_MAX_TABLES_PER_COMPONENT")]
    pub pool_max_tables_per_component: Option<u32>,
    /// Upper bound on the number of linear memories a single core module may
    /// define (`POOL_MAX_MEMORIES_PER_MODULE`). Unset leaves the Wasmtime
    /// default (1).
    #[env(from = "POOL_MAX_MEMORIES_PER_MODULE")]
    pub pool_max_memories_per_module: Option<u32>,
    /// Upper bound on the number of tables a single core module may define
    /// (`POOL_MAX_TABLES_PER_MODULE`). Unset leaves the Wasmtime default (1).
    #[env(from = "POOL_MAX_TABLES_PER_MODULE")]
    pub pool_max_tables_per_module: Option<u32>,
    /// Maximum size, in bytes, of a single core instance's `VMContext`
    /// metadata (`POOL_MAX_CORE_INSTANCE_SIZE`). Unset leaves the Wasmtime
    /// default (1 `MiB`).
    #[env(from = "POOL_MAX_CORE_INSTANCE_SIZE")]
    pub pool_max_core_instance_size: Option<usize>,
    /// Maximum size, in bytes, of a single component instance's metadata
    /// (`POOL_MAX_COMPONENT_INSTANCE_SIZE`). Unset leaves the Wasmtime default
    /// (1 `MiB`).
    #[env(from = "POOL_MAX_COMPONENT_INSTANCE_SIZE")]
    pub pool_max_component_instance_size: Option<usize>,
    /// Number of slots batched together when decommitting pooled memory to
    /// amortise syscalls (`POOL_DECOMMIT_BATCH_SIZE`). Unset leaves the
    /// Wasmtime default (1).
    #[env(from = "POOL_DECOMMIT_BATCH_SIZE")]
    pub pool_decommit_batch_size: Option<usize>,
    /// Whether to use the Linux `PAGEMAP_SCAN` ioctl to reset linear memory
    /// more cheaply on slot reuse (`POOL_PAGEMAP_SCAN`, one of `auto`/`yes`/
    /// `no`, default `no`). Requires Linux 6.7+; `auto` falls back to the
    /// default reset path where unsupported.
    #[env(from = "POOL_PAGEMAP_SCAN", default = "no", with = parse_enabled)]
    pub pool_pagemap_scan: Enabled,
    /// Whether the pooling allocator should use memory protection keys (MPK) to
    /// pack linear memories more densely (`POOL_MEMORY_PROTECTION_KEYS`, one of
    /// `auto`/`yes`/`no`, default `no`). Only effective on Linux/`x86_64` and
    /// only applied when built with the `mpk` feature; `auto` falls back cleanly
    /// where unsupported, and `yes` without the `mpk` feature is rejected at
    /// start-up.
    #[env(from = "POOL_MEMORY_PROTECTION_KEYS", default = "no", with = parse_enabled)]
    pub pool_memory_protection_keys: Enabled,
    /// Upper limit on how many memory protection keys the pooling allocator may
    /// allocate (`POOL_MAX_MEMORY_PROTECTION_KEYS`). Unset leaves the Wasmtime
    /// default. Only applied when built with the `mpk` feature.
    #[env(from = "POOL_MAX_MEMORY_PROTECTION_KEYS")]
    pub pool_max_memory_protection_keys: Option<usize>,
    /// Interval between background samples of pooling-allocator occupancy,
    /// emitted as `OpenTelemetry` gauges (`POOL_METRICS_INTERVAL_MS`, default
    /// 5000ms). `0` disables the sampler.
    #[env(from = "POOL_METRICS_INTERVAL_MS", default = "5000", with = parse_millis)]
    pub pool_metrics_interval: Duration,
    /// Whether to honour WebAssembly branch hints during compilation
    /// (`BRANCH_HINTING`, default `false`).
    #[env(from = "BRANCH_HINTING", default = "false")]
    pub branch_hinting: bool,
}

/// Build the [`Config`] shared by [`crate::compile`] and [`crate::DeploymentBuilder`].
///
/// Centralising it guarantees the compile-affecting settings (fuel metering,
/// branch hinting, memory reservation/guard size, and copy-on-write heap init)
/// are identical in the compile and load paths, so a pre-compiled component
/// stays loadable by [`wasmtime::component::Component::deserialize_file`].
/// Runtime-only settings are applied here too for a single source of truth.
impl From<&RuntimeOptions> for Config {
    fn from(options: &RuntimeOptions) -> Self {
        let mut config = Self::new();

        // Always enabled so each store can install an epoch deadline; the ticker
        // and per-store deadlines drive cooperative guest timeouts.
        config.epoch_interruption(true);

        // Copy-on-write heap images make per-request instantiation cheap. Pinned
        // on (the default) because it is compile-affecting: the compiling and
        // loading engines must agree, and an explicit value guards against a
        // future default change breaking artifact compatibility.
        config.memory_init_cow(true);

        if options.max_fuel > 0 {
            config.consume_fuel(true);
        }
        if options.branch_hinting {
            config.wasm_branch_hinting(true);
        }

        // Compile-affecting memory tunables (they drive bounds-check elision).
        // Applied only when set so an unset value preserves the Wasmtime
        // default; whatever is chosen must match between `omnia compile` and
        // `omnia run`.
        if let Some(bytes) = options.memory_reservation {
            config.memory_reservation(bytes);
        }
        if let Some(bytes) = options.memory_guard_size {
            config.memory_guard_size(bytes);
        }

        // Runtime-only engine settings (no artifact effect). Set before the
        // pooling early-return so they hold whether or not pooling is enabled.
        config.async_stack_zeroing(options.async_stack_zeroing);
        if let Some(bytes) = options.memory_reservation_for_growth {
            config.memory_reservation_for_growth(bytes);
        }

        // Guest backtraces are runtime-only (not artifact-affecting). Disable via
        // the non-deprecated max-frames API; `true` leaves the Wasmtime default
        // (backtraces on).
        if !options.wasm_backtrace {
            config.wasm_backtrace_max_frames(None);
        }

        if !options.pooling {
            return config;
        }

        // SECURITY: the pooling allocator + CoW is exactly the configuration
        // historical Wasmtime advisories target, so keeping the `46.0.x` pins
        // current matters most here (see the maintenance note in the workspace
        // `Cargo.toml`).
        let mut pool = PoolingAllocationConfig::new();

        // Totals are kept independent of the component-instance count: a single
        // component can transitively embed several core instances/memories/tables,
        // so pinning these to `pool_max_instances` would exhaust the memory/table
        // pools before reaching the advertised instance count under load.
        pool.total_component_instances(options.pool_max_instances)
            .total_core_instances(options.pool_total_core_instances)
            .total_memories(options.pool_total_memories)
            .total_tables(options.pool_total_tables)
            .total_stacks(options.pool_total_stacks)
            .max_memory_size(options.pool_max_memory_bytes.unwrap_or(options.max_memory_bytes))
            // Keep memory/tables/stacks resident across reuse to skip
            // decommit/zeroing; defaults of 0 preserve the prior behaviour.
            .linear_memory_keep_resident(options.pool_memory_keep_resident)
            .table_keep_resident(options.pool_table_keep_resident)
            .async_stack_keep_resident(options.pool_async_stack_keep_resident)
            .max_unused_warm_slots(options.pool_max_unused_warm_slots)
            // Linux-only fast linear-memory reset; the default (`No`) preserves the
            // prior behaviour and `Auto` falls back cleanly where unsupported.
            .pagemap_scan(options.pool_pagemap_scan);

        // GC heaps are only used by guests adopting the component-model GC /
        // reference types; current guests never allocate one. Gated behind the
        // opt-in `gc` feature, so the pool count is applied only when compiled in.
        cfg_if::cfg_if! {
            if #[cfg(feature = "gc")] {
                if let Some(count) = options.pool_total_gc_heaps {
                    pool.total_gc_heaps(count);
                }
            }
        }

        // Structural limits and sizes are applied only when explicitly set so an
        // unset value preserves the Wasmtime default.
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
        if let Some(size) = options.pool_decommit_batch_size {
            pool.decommit_batch_size(size);
        }

        cfg_if::cfg_if! {
            if #[cfg(feature = "mpk")] {
                pool.memory_protection_keys(options.pool_memory_protection_keys);
                if let Some(max) = options.pool_max_memory_protection_keys {
                    pool.max_memory_protection_keys(max);
                }
            }
        }

        config.allocation_strategy(InstanceAllocationStrategy::Pooling(pool));
        config
    }
}

impl RuntimeOptions {
    /// Finalize the runtime configuration.
    ///
    /// # Errors
    ///
    /// Returns an error if the runtime configuration cannot be loaded from the
    /// environment or fails cross-field validation.
    pub fn load() -> Result<Self> {
        let options = Self::from_env().finalize().map_err(anyhow::Error::from)?;
        options.validate()?;
        Ok(options)
    }

    /// Validate cross-field invariants that the per-field `FromEnv` parsing
    /// cannot express, surfacing a clear error before the engine is built
    /// (Wasmtime otherwise rejects the same combinations with a less specific
    /// message when the pool is constructed).
    ///
    /// # Errors
    ///
    /// Returns an error when the pooling configuration is internally
    /// inconsistent, e.g. a per-module structural limit exceeds the
    /// corresponding pool total.
    fn validate(&self) -> Result<()> {
        if !self.pooling {
            return Ok(());
        }

        // Fail fast on `yes` rather than silently ignoring MPK when the feature
        // that compiles in the Wasmtime support is absent.
        #[cfg(not(feature = "mpk"))]
        if self.pool_memory_protection_keys == Enabled::Yes {
            bail!("POOL_MEMORY_PROTECTION_KEYS=yes requires building omnia with the `mpk` feature");
        }

        // Likewise reject a GC heap count when the `gc` feature that compiles in
        // `total_gc_heaps` is absent, rather than silently ignoring it.
        #[cfg(not(feature = "gc"))]
        if self.pool_total_gc_heaps.is_some() {
            bail!("POOL_TOTAL_GC_HEAPS requires building omnia with the `gc` feature");
        }

        if let Some(per_module) = self.pool_max_memories_per_module
            && per_module > self.pool_total_memories
        {
            bail!(
                "POOL_MAX_MEMORIES_PER_MODULE ({per_module}) exceeds POOL_TOTAL_MEMORIES ({})",
                self.pool_total_memories
            );
        }
        if let Some(per_module) = self.pool_max_tables_per_module
            && per_module > self.pool_total_tables
        {
            bail!(
                "POOL_MAX_TABLES_PER_MODULE ({per_module}) exceeds POOL_TOTAL_TABLES ({})",
                self.pool_total_tables
            );
        }

        Ok(())
    }
}

/// Parse a millisecond count into a [`Duration`]; used by the `FromEnv` derive.
fn parse_millis(value: &str) -> ParseResult<Duration> {
    Ok(Duration::from_millis(value.parse::<u64>()?))
}

/// Parse the epoch tick, clamping to a 1ms minimum so the ticker interval can
/// never be zero; used by the `FromEnv` derive.
fn parse_tick(value: &str) -> ParseResult<Duration> {
    Ok(Duration::from_millis(value.parse::<u64>()?.max(1)))
}

/// Parse an `auto`/`yes`/`no` toggle into a [`wasmtime::Enabled`]; used by the
/// `FromEnv` derive for the pooling allocator's tri-state switches
/// (`PAGEMAP_SCAN`, memory protection keys).
fn parse_enabled(value: &str) -> ParseResult<Enabled> {
    match value.trim().to_ascii_lowercase().as_str() {
        "auto" => Ok(Enabled::Auto),
        "yes" | "true" | "on" | "1" => Ok(Enabled::Yes),
        "no" | "false" | "off" | "0" => Ok(Enabled::No),
        other => Err(format!("expected one of `auto`/`yes`/`no`, got `{other}`").into()),
    }
}

#[cfg(test)]
mod tests {
    use super::{Enabled, RuntimeOptions, parse_enabled};

    #[test]
    fn parse_enabled_values() {
        assert_eq!(parse_enabled("auto").unwrap(), Enabled::Auto);
        assert_eq!(parse_enabled("YES").unwrap(), Enabled::Yes);
        assert_eq!(parse_enabled(" no ").unwrap(), Enabled::No);
        parse_enabled("maybe").unwrap_err();
    }

    #[test]
    fn rejects_per_module_memories_over_total() {
        let options = RuntimeOptions {
            pool_total_memories: 8,
            pool_max_memories_per_module: Some(16),
            ..RuntimeOptions::load().expect("should load")
        };
        options.validate().unwrap_err();
    }

    #[test]
    fn rejects_per_module_tables_over_total() {
        let options = RuntimeOptions {
            pool_total_tables: 8,
            pool_max_tables_per_module: Some(16),
            ..RuntimeOptions::load().expect("should load")
        };
        options.validate().unwrap_err();
    }

    #[test]
    fn ignores_pool_limits_when_pooling_disabled() {
        let options = RuntimeOptions {
            pooling: false,
            pool_total_memories: 8,
            pool_max_memories_per_module: Some(16),
            ..RuntimeOptions::load().expect("should load")
        };
        options.validate().unwrap();
    }
}
