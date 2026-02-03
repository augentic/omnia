//! # WebAssembly Initiator

use std::env;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use qwasr_otel::Telemetry;
use tracing::instrument;
use wasmtime::component::{Component, InstancePre, Linker};
use wasmtime::{Config, Engine};
use wasmtime_wasi::WasiView;

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

    let mut config = Config::new();
    config.wasm_component_model_async(true);
    let engine = Engine::new(&config)?;

    // TODO: cause executing WebAssembly to periodically yield
    //  1. enable `Config::epoch_interruption`
    //  2. Set `Store::epoch_deadline_async_yield_and_update`
    //  3. Call `Engine::increment_epoch` periodically

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

    tracing::info!("runtime intialized");

    Ok(Compiled { component, linker })
}

/// A compiled WebAssembly component with its associated Linker.
pub struct Compiled<T: WasiView + 'static> {
    component: Component,
    linker: Linker<T>,
}

impl<T: WasiView> Compiled<T> {
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
