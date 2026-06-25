//! # WebAssembly Initiator

use std::collections::HashMap;
use std::env;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use omnia_otel::Telemetry;
use tracing::instrument;
use wasmtime::component::Linker;
use wasmtime::{Config, Engine};
use wasmtime_wasi::WasiView;

use crate::manifest::{Manifest, SourceSpec};
use crate::registry::{Guest, GuestId, Registry};
use crate::source::{FileSource, GuestSource, LoadedGuest};
use crate::{Host, RuntimeOptions};

/// Build a [`Compiled`] runtime, choosing single-file or manifest-driven
/// population.
///
/// Resolution: a `config` path (the `--config` flag or the `OMNI_CONFIG`
/// environment variable) selects a manifest-driven deployment; otherwise the
/// positional `wasm` path is the one-guest shorthand. At least one of the two
/// must be provided.
///
/// # Errors
///
/// Returns an error if neither a config nor a wasm path is available, or if the
/// selected source cannot be built.
pub async fn create_runtime<T: WasiView + 'static>(
    wasm: Option<PathBuf>, config: Option<PathBuf>,
) -> Result<Compiled<T>> {
    let config = config.or_else(|| env::var_os("OMNI_CONFIG").map(PathBuf::from));

    if let Some(config) = config {
        return create_from_manifest(&config).await;
    }

    let wasm = wasm.context(
        "no guest specified: pass a <wasm> path, or --config <omni.toml> (or set OMNI_CONFIG)",
    )?;
    create(&wasm).await
}

/// Build the Wasmtime `Engine` and `Linker` for a single-guest runtime.
///
/// This is the `omnia run <guest>.wasm` shorthand: load one component, derive
/// its identity from the file stem, and register it as the default guest — a
/// one-entry registry.
///
/// # Errors
///
/// Will fail if the provided `wasm` file cannot be compiled/deserialized as a
/// `Component` or the `Linker` cannot be initialized with WASI support.
#[instrument]
pub async fn create<T: WasiView + 'static>(wasm: &Path) -> Result<Compiled<T>> {
    let name = wasm.file_stem().and_then(|s| s.to_str()).unwrap_or("unknown");
    init_env(name)?;
    tracing::info!("initializing runtime");

    let (engine, linker, options) = engine_and_linker()?;

    let source = FileSource::new(wasm);
    let default = source.id().clone();
    let guests = source.load(&engine).await?;

    tracing::info!("runtime initialized");

    Ok(Compiled {
        engine,
        linker,
        options,
        guests,
        default,
    })
}

/// Build a runtime from a deployment manifest (`omni.toml`).
///
/// Resolves every `[[guest]]` source, builds the shared engine + linker, and
/// records the first guest as the default entry. Per-trigger capability routing
/// is layered on in a later phase.
///
/// # Errors
///
/// Will fail if the manifest cannot be loaded, if a guest uses a source kind not
/// yet supported, or if a guest component cannot be loaded.
#[instrument]
pub async fn create_from_manifest<T: WasiView + 'static>(manifest: &Path) -> Result<Compiled<T>> {
    let parsed = Manifest::load(manifest)?;

    // The default entry doubles as the telemetry/component name for now.
    let default = parsed
        .guests
        .first()
        .map_or_else(|| GuestId::from("omnia"), |entry| GuestId::from(entry.id.as_str()));
    init_env(default.as_str())?;
    tracing::info!("initializing runtime from manifest");

    let (engine, linker, options) = engine_and_linker()?;

    // Sources resolve relative to the manifest's directory.
    let base = manifest.parent().unwrap_or_else(|| Path::new("."));
    let mut guests = Vec::with_capacity(parsed.guests.len());
    for entry in &parsed.guests {
        let id = GuestId::from(entry.id.as_str());
        let source = match &entry.source {
            SourceSpec::Path(path) => {
                let resolved = if path.is_absolute() { path.clone() } else { base.join(path) };
                FileSource::with_id(id, resolved)
            }
            SourceSpec::Embedded(_) => {
                bail!("guest `{id}`: embedded sources are not yet supported (arrive with Phase 1b)")
            }
            SourceSpec::Oci(_) => {
                bail!("guest `{id}`: OCI sources are not yet supported")
            }
        };
        guests.extend(source.load(&engine).await?);
    }

    tracing::info!("runtime initialized from manifest");

    Ok(Compiled {
        engine,
        linker,
        options,
        guests,
        default,
    })
}

/// Build the shared engine, WASI-linked linker, and runtime options.
fn engine_and_linker<T: WasiView + 'static>() -> Result<(Engine, Linker<T>, RuntimeOptions)> {
    let options = RuntimeOptions::load()?;
    let engine = Engine::new(&Config::from(&options))?;

    // register services with runtime's Linker
    let mut linker = Linker::new(&engine);
    wasmtime_wasi::p2::add_to_linker_async(&mut linker)?;
    wasmtime_wasi::p3::add_to_linker(&mut linker)?;

    Ok((engine, linker, options))
}

/// A compiled set of WebAssembly components with their shared Linker, ready to
/// be assembled into a [`Registry`].
pub struct Compiled<T: WasiView + 'static> {
    engine: Engine,
    linker: Linker<T>,
    options: RuntimeOptions,
    guests: Vec<LoadedGuest>,
    default: GuestId,
}

impl<T: WasiView> Compiled<T> {
    /// Returns the environment-derived runtime options.
    #[must_use]
    pub const fn options(&self) -> &RuntimeOptions {
        &self.options
    }

    /// Link a WASI host's interfaces into the shared Linker.
    ///
    /// # Errors
    ///
    /// Will fail if the host cannot be added to the Linker.
    pub fn link<H: Host<T>>(&mut self, _: H) -> Result<()> {
        H::add_to_linker(&mut self.linker)
    }

    /// Pre-instantiate every loaded guest against the shared Linker and assemble
    /// the [`Registry`].
    ///
    /// Pre-instantiation happens once, here, after all hosts are linked; per call
    /// only a fresh instantiate on a new store remains.
    ///
    /// # Errors
    ///
    /// Will fail if a component cannot be pre-instantiated (e.g. an import is
    /// neither host-satisfied nor otherwise provided), or if the registry cannot
    /// be assembled.
    pub fn build_registry(&self) -> Result<Registry<T>> {
        let mut guests = HashMap::with_capacity(self.guests.len());
        for loaded in &self.guests {
            let instance_pre = self
                .linker
                .instantiate_pre(&loaded.component)
                .map_err(anyhow::Error::from)
                .with_context(|| format!("pre-instantiating guest `{}`", loaded.id))?;
            guests.insert(loaded.id.clone(), Guest::local(loaded.id.clone(), instance_pre));
        }

        Registry::new(self.engine.clone(), self.options.clone(), guests, self.default.clone())
    }
}

/// Initialize telemetry and the `COMPONENT` environment variable for the runtime.
///
/// # Errors
///
/// Will fail if the telemetry cannot be initialized.
fn init_env(name: &str) -> Result<()> {
    if env::var_os("COMPONENT").is_none() {
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
