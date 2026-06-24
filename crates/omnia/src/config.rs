//! # Runtime configuration
//!
//! Centralised, environment-driven configuration for the runtime engine and
//! per-guest stores.
//!
//! Keeping the compile-time [`Config`] in one place ([`compile_config`])
//! guarantees that the engine used to pre-compile a component
//! ([`crate::compile`]) and the engine used to load it ([`crate::create`])
//! agree on every code-affecting setting. This parity is required for
//! [`wasmtime::component::Component::deserialize_file`] to accept a
//! pre-compiled artifact.

use std::fmt::Display;
use std::str::FromStr;
use std::time::Duration;

use anyhow::{Result, anyhow};
use wasmtime::Config;

/// Default per-guest wall-clock timeout, in milliseconds.
const DEFAULT_GUEST_TIMEOUT_MS: u64 = 30_000;
/// Default epoch ticker interval, in milliseconds.
const DEFAULT_EPOCH_TICK_MS: u64 = 10;
/// Default per-guest linear-memory cap, in bytes (256 `MiB`).
const DEFAULT_MAX_MEMORY_BYTES: usize = 256 << 20;
/// Default maximum number of pooled instances.
const DEFAULT_POOL_MAX_INSTANCES: u32 = 1_000;

/// Runtime configuration loaded from the environment.
///
/// Values are read once at start-up via [`RuntimeConfig::from_env`] and then
/// threaded through the generated runtime to every store.
///
/// # Compile-time vs runtime settings
///
/// `max_fuel` and `branch_hinting` influence generated code, so they are
/// applied in [`compile_config`] and therefore must be identical when a
/// component is pre-compiled and when it is later loaded. The remaining values
/// only affect the engine or individual stores at runtime.
#[derive(Clone, Debug)]
pub struct RuntimeConfig {
    /// Wall-clock cap applied to a single guest invocation.
    pub guest_timeout: Duration,
    /// Interval between [`wasmtime::Engine::increment_epoch`] ticks. Also the
    /// granularity at which CPU-bound guests yield to the async executor.
    pub epoch_tick: Duration,
    /// Maximum linear-memory size, in bytes, a guest may grow to.
    pub max_memory_bytes: usize,
    /// Per-invocation fuel budget; `0` disables fuel metering.
    pub max_fuel: u64,
    /// Whether the pooling instance allocator is enabled.
    pub pooling: bool,
    /// Maximum number of instances held by the pooling allocator.
    pub pool_max_instances: u32,
    /// Maximum linear-memory size, in bytes, reserved per pooled memory.
    pub pool_max_memory_bytes: usize,
    /// Whether to honour WebAssembly branch hints during compilation.
    pub branch_hinting: bool,
}

impl Default for RuntimeConfig {
    fn default() -> Self {
        Self {
            guest_timeout: Duration::from_millis(DEFAULT_GUEST_TIMEOUT_MS),
            epoch_tick: Duration::from_millis(DEFAULT_EPOCH_TICK_MS),
            max_memory_bytes: DEFAULT_MAX_MEMORY_BYTES,
            max_fuel: 0,
            pooling: true,
            pool_max_instances: DEFAULT_POOL_MAX_INSTANCES,
            pool_max_memory_bytes: DEFAULT_MAX_MEMORY_BYTES,
            branch_hinting: false,
        }
    }
}

impl RuntimeConfig {
    /// Load the runtime configuration from `OMNIA_*` environment variables,
    /// falling back to conservative defaults.
    ///
    /// Recognised variables (booleans use `true`/`false`):
    ///
    /// - `OMNIA_GUEST_TIMEOUT_MS` (default `30000`)
    /// - `OMNIA_EPOCH_TICK_MS` (default `10`)
    /// - `OMNIA_MAX_MEMORY_BYTES` (default `268435456`)
    /// - `OMNIA_MAX_FUEL` (default `0`, disabled)
    /// - `OMNIA_POOLING` (default `true`)
    /// - `OMNIA_POOL_MAX_INSTANCES` (default `1000`)
    /// - `OMNIA_POOL_MAX_MEMORY_BYTES` (default: `OMNIA_MAX_MEMORY_BYTES`)
    /// - `OMNIA_BRANCH_HINTING` (default `false`)
    ///
    /// # Errors
    ///
    /// Returns an error if a recognised variable is set but cannot be parsed.
    pub fn from_env() -> Result<Self> {
        let max_memory_bytes = env_parse("OMNIA_MAX_MEMORY_BYTES", DEFAULT_MAX_MEMORY_BYTES)?;
        let tick_ms = env_parse("OMNIA_EPOCH_TICK_MS", DEFAULT_EPOCH_TICK_MS)?.max(1);

        Ok(Self {
            guest_timeout: Duration::from_millis(env_parse(
                "OMNIA_GUEST_TIMEOUT_MS",
                DEFAULT_GUEST_TIMEOUT_MS,
            )?),
            epoch_tick: Duration::from_millis(tick_ms),
            max_memory_bytes,
            max_fuel: env_parse("OMNIA_MAX_FUEL", 0_u64)?,
            pooling: env_parse("OMNIA_POOLING", true)?,
            pool_max_instances: env_parse("OMNIA_POOL_MAX_INSTANCES", DEFAULT_POOL_MAX_INSTANCES)?,
            pool_max_memory_bytes: env_parse("OMNIA_POOL_MAX_MEMORY_BYTES", max_memory_bytes)?,
            branch_hinting: env_parse("OMNIA_BRANCH_HINTING", false)?,
        })
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
    /// [`wasmtime::component::Component::deserialize_file`] will reject the
    /// artifact.
    #[must_use]
    pub fn compile(&self) -> Config {
        let mut config = Config::new();

        // Always enabled so each store can install an epoch deadline; the ticker
        // and per-store deadlines drive cooperative guest timeouts.
        config.epoch_interruption(true);

        if self.max_fuel > 0 {
            config.consume_fuel(true);
        }
        if self.branch_hinting {
            config.wasm_branch_hinting(true);
        }

        config
    }
}

/// Parse the environment variable `key`, returning `default` when it is unset.
fn env_parse<T>(key: &str, default: T) -> Result<T>
where
    T: FromStr,
    T::Err: Display,
{
    std::env::var(key).map_or_else(
        |_| Ok(default),
        |value| value.parse::<T>().map_err(|e| anyhow!("invalid {key}: {e}")),
    )
}
