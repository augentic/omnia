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

use crate::manifest::{Manifest, RouteSpec, SourceSpec};
use crate::registry::{Guest, GuestId, Registry};
use crate::routing::{HttpRoutes, Routess, TopicRoutes};
use crate::source::{EmbeddedSource, FileSource, GuestSource, LoadedGuest};
use crate::{Host, RuntimeOptions};

/// Build a [`Compiled`] runtime, choosing single-file or manifest-driven
/// population.
///
/// Resolution: a `config` path (the `--config` flag or the `OMNI_CONFIG`
/// environment variable) selects a manifest-driven deployment; otherwise the
/// positional `wasm` path is the one-guest shorthand. At least one of the two
/// must be provided.
///
/// The `embedded` map carries build-time `include_bytes!` blobs (declared in
/// the `runtime!` macro) that a manifest's `source.embedded = "<name>"` may
/// activate; the single-file shorthand never consults it.
///
/// # Errors
///
/// Returns an error if neither a config nor a wasm path is available, or if the
/// selected source cannot be built.
pub async fn create_runtime<T: WasiView + 'static>(
    wasm: Option<PathBuf>, config: Option<PathBuf>,
    embedded: &'static [(&'static str, &'static [u8])],
) -> Result<Compiled<T>> {
    let config = config.or_else(|| env::var_os("OMNI_CONFIG").map(PathBuf::from));

    if let Some(config) = config {
        return create_from_manifest(&config, embedded).await;
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
        // The single-file shorthand carries no routes: its sole guest is the
        // catch-all for every trigger it can answer.
        routes: Routess::default(),
    })
}

/// Build a runtime from a deployment manifest (`omni.toml`).
///
/// Resolves every `[[guest]]` source (file or embedded), builds the shared
/// engine + linker, records the first guest as the default entry, and assembles
/// the per-trigger route tables from the `[[route.*]]` sections.
///
/// # Errors
///
/// Will fail if the manifest cannot be loaded, if a guest names an embedded blob
/// not declared in `runtime!`, if a guest uses a source kind not yet supported,
/// or if a guest component cannot be loaded.
#[instrument(skip(embedded))]
pub async fn create_from_manifest<T: WasiView + 'static>(
    manifest: &Path, embedded: &'static [(&'static str, &'static [u8])],
) -> Result<Compiled<T>> {
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
        let loaded = match &entry.source {
            SourceSpec::Path(path) => {
                let resolved = if path.is_absolute() { path.clone() } else { base.join(path) };
                FileSource::with_id(id, resolved).load(&engine).await?
            }
            SourceSpec::Embedded(name) => {
                let bytes = lookup_embedded(embedded, name).with_context(|| {
                    format!("guest `{id}`: embedded guest `{name}` is not declared in `runtime!`")
                })?;
                EmbeddedSource::new(id, bytes).load(&engine).await?
            }
            SourceSpec::Oci(_) => {
                bail!("guest `{id}`: OCI sources are not yet supported")
            }
        };
        guests.extend(loaded);
    }

    let routes = route_tables(&parsed.route);

    tracing::info!("runtime initialized from manifest");

    Ok(Compiled {
        engine,
        linker,
        options,
        guests,
        default,
        routes,
    })
}

/// Resolve an embedded guest's bytes by name from the build-time map declared
/// in the `runtime!` macro.
fn lookup_embedded(
    embedded: &'static [(&'static str, &'static [u8])], name: &str,
) -> Option<&'static [u8]> {
    embedded.iter().find(|(declared, _)| *declared == name).map(|(_, bytes)| *bytes)
}

/// Convert the manifest's parsed routes into the registry's `GuestId`-typed,
/// per-trigger route tables.
fn route_tables(spec: &RouteSpec) -> Routess {
    let http = HttpRoutes::new(
        spec.http.iter().map(|e| (e.prefix.clone(), GuestId::from(e.guest.as_str()))),
    );
    let messaging = TopicRoutes::new(
        spec.messaging.iter().map(|e| (e.topic.clone(), GuestId::from(e.guest.as_str()))),
    );
    let websocket = TopicRoutes::new(
        spec.websocket.iter().map(|e| (e.topic.clone(), GuestId::from(e.guest.as_str()))),
    );
    Routess::new(http, messaging, websocket)
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
    routes: Routess,
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

        Registry::new(
            self.engine.clone(),
            self.options.clone(),
            guests,
            self.default.clone(),
            self.routes.clone(),
        )
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
    fn lookup_embedded_by_name() {
        // Mirrors the `(&str, &[u8])` shape the `runtime!` macro emits.
        const A: &[u8] = b"component-a";
        const B: &[u8] = b"component-b";
        const EMBEDDED: &[(&str, &[u8])] = &[("a", A), ("b", B)];

        assert_eq!(super::lookup_embedded(EMBEDDED, "b"), Some(B));
        assert_eq!(super::lookup_embedded(EMBEDDED, "missing"), None);
    }

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
