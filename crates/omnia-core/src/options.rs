//! # Runtime configuration
//!
//! Centralised, environment-driven configuration for the runtime engine and
//! per-guest stores.
//!
//! The compile-affecting settings are one value, [`CompileOptions`], applied
//! by one body, [`CompileOptions::configure`]: the ahead-of-time compiler
//! takes it explicitly, and the runtime's [`Config`] (the
//! `From<&RuntimeOptions>` conversion) applies the copy it loaded from the
//! environment. That parity is what lets the loading engine accept a
//! pre-compiled artifact.

// `derive(FromEnv)` generates undocumented associated functions on a type the
// crate root re-exports
#![allow(missing_docs)]

use std::time::Duration;

use anyhow::{Result, bail};
use fromenv::{FromEnv, ParseResult};
use wasmtime::{Config, Enabled, InstanceAllocationStrategy, PoolingAllocationConfig};

/// Runtime configuration loaded from the environment.
///
/// Read once at start-up and threaded through the runtime to every store.
/// Each field maps to one environment variable (booleans use `true`/`false`);
/// [`RuntimeOptions::requirements`] prints the full list with defaults.
///
/// The fields marked compile-affecting are the [`CompileOptions`] this
/// runtime loads artifacts under ([`compile_options`](Self::compile_options))
/// and must match between pre-compiling a component and loading it. The rest
/// only affect the engine or individual stores at runtime.
// flat by design: each boolean is one environment variable
#[allow(clippy::struct_excessive_bools)]
#[derive(Clone, Debug, FromEnv)]
pub struct RuntimeOptions {
    /// Wall-clock cap on a server or server-rooted link-dispatch invocation (`GUEST_TIMEOUT_MS`, default 30s, min 1ms; a command-mode chain, including its link hops, is uncapped).
    #[env(from = "GUEST_TIMEOUT_MS", default = "30000", with = parse_timeout)]
    pub guest_timeout: Duration,
    /// Epoch-increment interval, also the CPU-bound guest yield granularity (`EPOCH_TICK_MS`, default 10ms, min 1ms).
    #[env(from = "EPOCH_TICK_MS", default = "10", with = parse_tick)]
    pub epoch_tick: Duration,
    /// Maximum linear-memory size a guest may grow to, in bytes (`MAX_MEMORY_BYTES`, default 256 `MiB`).
    #[env(from = "MAX_MEMORY_BYTES", default = "268435456")]
    pub max_memory_bytes: usize,
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
    /// Maximum core instances held by the pooling allocator (`POOL_TOTAL_CORE_INSTANCES`, default 1000).
    #[env(from = "POOL_TOTAL_CORE_INSTANCES", default = "1000")]
    pub pool_total_core_instances: u32,
    /// Maximum linear memories held by the pooling allocator (`POOL_TOTAL_MEMORIES`, default 1000).
    #[env(from = "POOL_TOTAL_MEMORIES", default = "1000")]
    pub pool_total_memories: u32,
    /// Maximum tables held by the pooling allocator (`POOL_TOTAL_TABLES`, default 1000).
    #[env(from = "POOL_TOTAL_TABLES", default = "1000")]
    pub pool_total_tables: u32,
    /// Maximum async stacks held by the pooling allocator (`POOL_TOTAL_STACKS`, default 1000).
    #[env(from = "POOL_TOTAL_STACKS", default = "1000")]
    pub pool_total_stacks: u32,
    /// Maximum GC heaps held by the pooling allocator; requires the `gc` feature (`POOL_TOTAL_GC_HEAPS`, unset: Wasmtime default).
    #[env(from = "POOL_TOTAL_GC_HEAPS")]
    pub pool_total_gc_heaps: Option<u32>,
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
    /// Pack linear memories with memory protection keys; requires the `mpk` feature (`POOL_MEMORY_PROTECTION_KEYS`, `auto`/`yes`/`no`, default `no`).
    #[env(from = "POOL_MEMORY_PROTECTION_KEYS", default = "no", with = parse_enabled)]
    pub pool_memory_protection_keys: Enabled,
    /// Upper limit on MPK keys the pool may allocate; requires the `mpk` feature (`POOL_MAX_MEMORY_PROTECTION_KEYS`, unset: Wasmtime default).
    #[env(from = "POOL_MAX_MEMORY_PROTECTION_KEYS")]
    pub pool_max_memory_protection_keys: Option<usize>,
    /// Interval between pooling-occupancy metric samples; `0` disables (`POOL_METRICS_INTERVAL_MS`, default 5000ms).
    #[env(from = "POOL_METRICS_INTERVAL_MS", default = "5000", with = parse_millis)]
    pub pool_metrics_interval: Duration,
    /// Honour WebAssembly branch hints during compilation; compile-affecting (`BRANCH_HINTING`, default `false`).
    #[env(from = "BRANCH_HINTING", default = "false")]
    pub branch_hinting: bool,
}

/// The settings that shape a compiled artifact.
///
/// A pre-compiled component loads only into an engine configured with the
/// values it was compiled under, so these travel as one value. The default
/// matches the environment defaults, so an artifact compiled with
/// `CompileOptions::default()` loads into a runtime that sets none of the
/// compile-affecting variables.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CompileOptions {
    /// Per-invocation fuel budget; `0` disables metering (`MAX_FUEL`).
    pub max_fuel: u64,
    /// Honour WebAssembly branch hints (`BRANCH_HINTING`).
    pub branch_hinting: bool,
    /// Virtual address space reserved per linear memory, in bytes; `None`
    /// is Wasmtime's default (`MEMORY_RESERVATION`).
    pub memory_reservation: Option<u64>,
    /// Unmapped guard region after each linear memory, in bytes; `None` is
    /// Wasmtime's default (`MEMORY_GUARD_SIZE`).
    pub memory_guard_size: Option<u64>,
    /// Emit ELF symbol tables, for profilers and `wasmtime objdump`
    /// (`DEBUG_SYMBOLS`).
    pub debug_symbols: bool,
    /// Record the machine-code-to-wasm-offset map that gives traps and
    /// backtraces their wasm offsets (`GENERATE_ADDRESS_MAP`).
    pub generate_address_map: bool,
}

impl Default for CompileOptions {
    fn default() -> Self {
        Self {
            max_fuel: 0,
            branch_hinting: false,
            memory_reservation: None,
            memory_guard_size: None,
            debug_symbols: false,
            generate_address_map: true,
        }
    }
}

impl CompileOptions {
    /// Apply every compile-affecting setting to `config` — these six, and the
    /// two the runtime fixes (epoch interruption, copy-on-write heap images).
    ///
    /// The one body both the ahead-of-time compiler and the runtime's
    /// [`Config`] go through, so the two engines cannot disagree.
    pub fn configure(&self, config: &mut Config) {
        // epoch interruption: per-store deadlines drive cooperative guest timeouts
        config.epoch_interruption(true);

        // cow heap images are compile-affecting, so pinned explicitly
        config.memory_init_cow(true);

        if self.max_fuel > 0 {
            config.consume_fuel(true);
        }
        if self.branch_hinting {
            config.wasm_branch_hinting(true);
        }

        // memory tunables only when set, so unset keeps the wasmtime default
        if let Some(bytes) = self.memory_reservation {
            config.memory_reservation(bytes);
        }
        if let Some(bytes) = self.memory_guard_size {
            config.memory_guard_size(bytes);
        }

        // artifact size: strip symbols, keep the address map for trap offsets
        config.debug_symbols(self.debug_symbols);
        config.generate_address_map(self.generate_address_map);
    }
}

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

        // totals stay independent of the instance count: one component can
        // embed several core instances, memories and tables
        pool.total_component_instances(options.pool_max_instances)
            .total_core_instances(options.pool_total_core_instances)
            .total_memories(options.pool_total_memories)
            .total_tables(options.pool_total_tables)
            .total_stacks(options.pool_total_stacks)
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

        // gc heaps only when compiled in
        cfg_if::cfg_if! {
            if #[cfg(feature = "gc")] {
                if let Some(count) = options.pool_total_gc_heaps {
                    pool.total_gc_heaps(count);
                }
            }
        }

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
    pub fn load_env() -> Result<Self> {
        let options = Self::from_env().finalize().map_err(anyhow::Error::from)?;
        options.validate()?;
        Ok(options)
    }

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

    // Cross-field invariants `FromEnv` cannot express, checked before the
    // engine is built where wasmtime's own rejection is less specific.
    fn validate(&self) -> Result<()> {
        if !self.pooling {
            return Ok(());
        }

        // fail fast on a feature the build lacks rather than ignore it
        if !cfg!(feature = "mpk") && self.pool_memory_protection_keys == Enabled::Yes {
            bail!("POOL_MEMORY_PROTECTION_KEYS=yes requires building omnia with the `mpk` feature");
        }
        if !cfg!(feature = "gc") && self.pool_total_gc_heaps.is_some() {
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

fn parse_millis(value: &str) -> ParseResult<Duration> {
    Ok(Duration::from_millis(value.parse::<u64>()?))
}

// a zero wall-clock bound would time out every invocation
fn parse_timeout(value: &str) -> ParseResult<Duration> {
    let millis = value.parse::<u64>()?;
    if millis == 0 {
        return Err("must be at least 1 (a zero timeout would fail every invocation)".into());
    }
    Ok(Duration::from_millis(millis))
}

// clamped so the ticker interval can never be zero
fn parse_tick(value: &str) -> ParseResult<Duration> {
    Ok(Duration::from_millis(value.parse::<u64>()?.max(1)))
}

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
    use super::{CompileOptions, Enabled, RuntimeOptions, parse_enabled};

    // an artifact compiled under the defaults loads into a runtime whose
    // environment sets none of the compile-affecting variables
    #[test]
    fn compile_options_default_is_env_default() {
        let options = RuntimeOptions::load_env().expect("should load");
        assert_eq!(options.compile_options(), CompileOptions::default());
    }

    #[test]
    fn parse_enabled_values() {
        assert_eq!(parse_enabled("auto").unwrap(), Enabled::Auto);
        assert_eq!(parse_enabled("YES").unwrap(), Enabled::Yes);
        assert_eq!(parse_enabled(" no ").unwrap(), Enabled::No);
        parse_enabled("maybe").unwrap_err();
    }

    #[test]
    fn per_module_memories_over_total() {
        let options = RuntimeOptions {
            pool_total_memories: 8,
            pool_max_memories_per_module: Some(16),
            ..RuntimeOptions::load_env().expect("should load")
        };
        options.validate().unwrap_err();
    }

    #[test]
    fn per_module_tables_over_total() {
        let options = RuntimeOptions {
            pool_total_tables: 8,
            pool_max_tables_per_module: Some(16),
            ..RuntimeOptions::load_env().expect("should load")
        };
        options.validate().unwrap_err();
    }

    #[test]
    fn pooling_disabled() {
        let options = RuntimeOptions {
            pooling: false,
            pool_total_memories: 8,
            pool_max_memories_per_module: Some(16),
            ..RuntimeOptions::load_env().expect("should load")
        };
        options.validate().unwrap();
    }
}
