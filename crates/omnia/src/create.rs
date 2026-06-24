//! # WebAssembly Initiator

use std::env;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use omnia_otel::Telemetry;
use tracing::instrument;
use wasmtime::component::{Component, InstancePre, Linker};
use wasmtime::{Config, Engine};
use wasmtime_wasi::WasiView;

use crate::RuntimeOptions;
use crate::traits::Host;

/// Build the Wasmtime `Engine` and `Linker` for this runtime.
///
/// # Errors
///
/// Will fail if the provided `wasm` file cannot be compiled/deserialized
/// as a `Component` or the `Linker` cannot be initialized with WASI
/// support.
#[instrument]
pub fn create<T: WasiView + 'static>(wasm: &PathBuf) -> Result<Compiled<T>> {
    init_env(wasm)?;
    tracing::info!("initializing runtime");

    let options = RuntimeOptions::load()?;
    let engine = Engine::new(&Config::from(&options))?;

    // SAFETY: The caller should ensure only valid pre-compiled wasm files are provided.
    let component = unsafe { Component::deserialize_file(&engine, wasm) }.or_else(|e| {
        if cfg!(feature = "jit") {
            Component::from_file(&engine, wasm)
        } else {
            Err(wasmtime::Error::msg(format!(
                "Issue loading component: {e}. Enable `jit` feature to load wasm32 files."
            )))
        }
    })?;

    // register services with runtime's Linker
    let mut linker = Linker::new(&engine);
    wasmtime_wasi::p2::add_to_linker_async(&mut linker)?;
    wasmtime_wasi::p3::add_to_linker(&mut linker)?;

    tracing::info!("runtime initialized");

    Ok(Compiled {
        component,
        linker,
        options,
    })
}

/// A compiled WebAssembly component with its associated Linker.
pub struct Compiled<T: WasiView + 'static> {
    component: Component,
    linker: Linker<T>,
    options: RuntimeOptions,
}

impl<T: WasiView> Compiled<T> {
    /// Returns the environment-derived runtime options.
    #[must_use]
    pub const fn options(&self) -> &RuntimeOptions {
        &self.options
    }

    /// Link a WASI component to the runtime.
    ///
    /// # Errors
    ///
    /// Will fail if the host cannot be added to the Linker.
    pub fn link<H: Host<T>>(&mut self, _: H) -> Result<()> {
        H::add_to_linker(&mut self.linker)
    }

    /// Pre-instantiate component.
    ///
    /// # Errors
    ///
    /// Will fail if the component cannot be pre-instantiated.
    pub fn pre_instantiate(&mut self) -> Result<InstancePre<T>> {
        self.linker.instantiate_pre(&self.component).map_err(anyhow::Error::from)
    }
}

/// Initialize telemetry for the runtime.
///
/// # Errors
///
/// Will fail if the telemetry cannot be initialized.
fn init_env(wasm: &Path) -> Result<()> {
    let name = wasm.file_stem().and_then(|s| s.to_str()).unwrap_or("unknown");

    if env::var("COMPONENT").is_err() {
        // SAFETY: Environment variable modification is safe here because:
        // 1. This runs during single-threaded initialization
        // 2. Backend clients that depend on these vars are created after this
        unsafe {
            env::set_var("COMPONENT", name);
        };
    }

    // telemetry
    let mut builder = Telemetry::new(name);
    if let Ok(endpoint) = env::var("OTEL_GRPC_URL") {
        builder = builder.endpoint(endpoint);
    }
    builder.build().context("initializing telemetry")
}

#[cfg(test)]
mod tests {
    use wasmtime::{Config, Engine};

    use crate::RuntimeOptions;

    #[test]
    fn builds_with_defaults() {
        Engine::new(&Config::from(&RuntimeOptions::load().expect("should load")))
            .expect("default pooling config should build an engine");
    }

    #[test]
    fn builds_with_pooling() {
        // Independent totals plus per-component/per-module limits, sized small
        // (and with a tiny per-memory cap) so the reservation stays cheap.
        let options = RuntimeOptions {
            pool_max_instances: 8,
            pool_total_core_instances: 8,
            pool_total_memories: 16,
            pool_total_tables: 16,
            pool_total_stacks: 8,
            pool_max_memory_bytes: Some(1 << 20),
            pool_max_memories_per_component: Some(4),
            pool_max_tables_per_component: Some(4),
            pool_max_memories_per_module: Some(2),
            pool_max_tables_per_module: Some(2),
            pool_decommit_batch_size: Some(8),
            ..RuntimeOptions::load().expect("should load")
        };
        Engine::new(&Config::from(&options))
            .expect("decoupled multi-memory pooling config should build an engine");
    }

    #[test]
    fn builds_without_pooling() {
        let options = RuntimeOptions {
            pooling: false,
            ..RuntimeOptions::load().expect("should load")
        };
        Engine::new(&Config::from(&options)).expect("non-pooling config should build an engine");
    }
}
