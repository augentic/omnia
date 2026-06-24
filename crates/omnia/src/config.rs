//! # Runtime configuration
//!
//! Centralised, environment-driven configuration for the runtime engine and
//! per-guest stores.
//!
//! Building the compile-time [`Config`] in one place (the
//! `From<&RuntimeConfig>` conversion) guarantees that the engine used to
//! pre-compile a component ([`crate::compile`]) and the engine used to load it
//! ([`crate::create`]) agree on every code-affecting setting. This parity is
//! required for [`wasmtime::component::Component::deserialize_file`] to accept a
//! pre-compiled artifact.

// `derive(FromEnv)` generates undocumented `from_env`/`requirements` associated
// functions; `RuntimeConfig` is re-exported from the crate root so they would
// otherwise trip `missing_docs`.
#![allow(missing_docs)]

use std::time::Duration;

use anyhow::Result;
use fromenv::{FromEnv, ParseResult};
use wasmtime::Config;

/// Runtime configuration loaded from the environment.
///
/// Values are read once at start-up via `RuntimeConfig::from_env().finalize()`
/// and threaded through the generated runtime to every store. Each field maps to
/// an `OMNIA_*` environment variable (booleans use `true`/`false`); call
/// [`RuntimeConfig::requirements`] to print the full list with defaults.
///
/// # Compile-time vs runtime settings
///
/// `max_fuel` and `branch_hinting` influence generated code, so they are applied
/// by the [`Config`] conversion and therefore must be identical when a component
/// is pre-compiled and when it is later loaded. The remaining values only affect
/// the engine or individual stores at runtime.
#[derive(Clone, Debug, FromEnv)]
pub struct RuntimeConfig {
    /// Wall-clock cap applied to a single guest invocation
    /// (`OMNIA_GUEST_TIMEOUT_MS`, default 30s).
    #[env(from = "OMNIA_GUEST_TIMEOUT_MS", default = "30000", with = parse_millis)]
    pub guest_timeout: Duration,
    /// Interval between [`wasmtime::Engine::increment_epoch`] ticks, and the
    /// granularity at which CPU-bound guests yield to the async executor
    /// (`OMNIA_EPOCH_TICK_MS`, default 10ms, clamped to a 1ms minimum).
    #[env(from = "OMNIA_EPOCH_TICK_MS", default = "10", with = parse_tick)]
    pub epoch_tick: Duration,
    /// Maximum linear-memory size, in bytes, a guest may grow to
    /// (`OMNIA_MAX_MEMORY_BYTES`, default 256 `MiB`).
    #[env(from = "OMNIA_MAX_MEMORY_BYTES", default = "268435456")]
    pub max_memory_bytes: usize,
    /// Per-invocation fuel budget; `0` disables fuel metering
    /// (`OMNIA_MAX_FUEL`, default disabled).
    #[env(from = "OMNIA_MAX_FUEL", default = "0")]
    pub max_fuel: u64,
    /// Whether the pooling instance allocator is enabled
    /// (`OMNIA_POOLING`, default `true`).
    #[env(from = "OMNIA_POOLING", default = "true")]
    pub pooling: bool,
    /// Maximum number of instances held by the pooling allocator
    /// (`OMNIA_POOL_MAX_INSTANCES`, default 1000).
    #[env(from = "OMNIA_POOL_MAX_INSTANCES", default = "1000")]
    pub pool_max_instances: u32,
    /// Maximum linear-memory size, in bytes, reserved per pooled memory
    /// (`OMNIA_POOL_MAX_MEMORY_BYTES`). When unset it inherits
    /// `max_memory_bytes`.
    #[env(from = "OMNIA_POOL_MAX_MEMORY_BYTES")]
    pub pool_max_memory_bytes: Option<usize>,
    /// Whether to honour WebAssembly branch hints during compilation
    /// (`OMNIA_BRANCH_HINTING`, default `false`).
    #[env(from = "OMNIA_BRANCH_HINTING", default = "false")]
    pub branch_hinting: bool,
}

/// Build the compile-time [`Config`] shared by [`crate::compile`] and
/// [`crate::create`].
///
/// Only settings that influence generated code belong here so that a
/// pre-compiled component remains loadable. The component-model-async feature
/// and WASI 0.3.0 are enabled by default in Wasmtime 46, so they are no longer
/// set explicitly.
///
/// # Compile/run parity
///
/// `OMNIA_MAX_FUEL` (which enables fuel metering) and `OMNIA_BRANCH_HINTING`
/// change the compiled artifact. They must hold the same value when a component
/// is pre-compiled with `omnia compile` and when it is later run, otherwise
/// [`wasmtime::component::Component::deserialize_file`] will reject the artifact.
impl From<&RuntimeConfig> for Config {
    fn from(rc: &RuntimeConfig) -> Self {
        let mut config = Self::new();

        // Always enabled so each store can install an epoch deadline; the ticker
        // and per-store deadlines drive cooperative guest timeouts.
        config.epoch_interruption(true);

        if rc.max_fuel > 0 {
            config.consume_fuel(true);
        }
        if rc.branch_hinting {
            config.wasm_branch_hinting(true);
        }

        config
    }
}

impl RuntimeConfig {
    /// Finalize the runtime configuration.
    ///
    /// # Errors
    ///
    /// Returns an error if the runtime configuration cannot be loaded from the
    /// environment.
    pub fn load() -> Result<Self> {
        Self::from_env().finalize().map_err(anyhow::Error::from)
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
