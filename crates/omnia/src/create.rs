//! # WebAssembly Initiator

use std::collections::{BTreeSet, HashMap};
use std::env;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use anyhow::{Context, Result, bail};
use omnia_otel::Telemetry;
use tracing::instrument;
use wasmtime::component::Linker;
use wasmtime::{Config, Engine};
use wasmtime_wasi::WasiView;
use wrpc_wasmtime::WrpcView;

use crate::dispatch::{DispatchHandle, link_dynamic};
use crate::manifest::{Manifest, RouteSpec, SourceSpec};
use crate::registry::{Guest, GuestId, Registry};
use crate::routing::{HttpRoutes, Routes, TopicRoutes};
use crate::selector::FirstArgSelector;
use crate::source::{FileSource, GuestSource, LoadedGuest};
use crate::{Host, RuntimeOptions};

// /// Build the Wasmtime `Engine` and `Linker` for a single-guest runtime.
// ///
// /// This is the `omnia run <guest>.wasm` shorthand: load one component, derive
// /// its identity from the file stem, and register it as the sole entry — a
// /// one-entry registry.
// ///
// /// # Errors
// ///
// /// Will fail if the provided `wasm` file cannot be compiled/deserialized as a
// /// `Component` or the `Linker` cannot be initialized with WASI support.
// #[instrument]
// pub async fn create<T: WasiView + 'static>(wasm: &Path) -> Result<Compiled<T>> {
//     let name = wasm.file_stem().and_then(|s| s.to_str()).unwrap_or("unknown");
//     init_env(name)?;
//     tracing::info!("initializing runtime");

//     let (engine, linker, options) = engine_and_linker()?;

//     let source = FileSource::new(wasm);
//     let guests = source.load(&engine).await?;

//     tracing::info!("runtime initialized");

//     Ok(Compiled {
//         engine,
//         linker,
//         options,
//         guests,
//         // The single-file shorthand carries no routes: its sole guest is the
//         // catch-all for every trigger it can answer.
//         routes: Routes::default(),
//         // ...and no host-mediated links: one guest has nobody to dispatch to.
//         link_interfaces: BTreeSet::new(),
//     })
// }

/// Selects where a runtime's guests come from, then [`compile`]s them into a
/// [`Compiled`] runtime ready for host linking.
///
/// The single-file shorthand ([`wasm`]) and the manifest-driven deployment
/// ([`config`]) are both expressed here; [`compile`] resolves whichever is set —
/// falling back to the `OMNI_CONFIG` environment variable for the manifest.
///
/// ```ignore
/// let compiled = RegistryBuilder::new()
///     .wasm(wasm)
///     .config(config)
///     .compile::<StoreCtx>()
///     .await?;
/// ```
///
/// [`wasm`]: Self::wasm
/// [`config`]: Self::config
/// [`compile`]: Self::compile
#[derive(Debug, Default)]
pub struct RegistryBuilder {
    wasm: Option<PathBuf>,
    config: Option<PathBuf>,
}

impl RegistryBuilder {
    /// Start a new builder with no source selected.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Set the single-guest `wasm` path — the `omnia run <wasm>` shorthand.
    #[must_use]
    pub fn wasm(mut self, wasm: impl Into<Option<PathBuf>>) -> Self {
        self.wasm = wasm.into();
        self
    }

    /// Set the deployment manifest (`omni.toml`) path for a multi-guest
    /// deployment.
    #[must_use]
    pub fn config(mut self, config: impl Into<Option<PathBuf>>) -> Self {
        self.config = config.into();
        self
    }

    /// Resolve the configured source into a [`Compiled`] runtime, choosing
    /// single-file or manifest-driven population.
    ///
    /// Resolution: a `config` path (set via [`config`](Self::config) or the
    /// `OMNI_CONFIG` environment variable) selects a manifest-driven deployment;
    /// otherwise the `wasm` path is the one-guest shorthand. At least one of the
    /// two must be provided.
    ///
    /// # Errors
    ///
    /// Returns an error if neither a config nor a wasm path is available, or if
    /// the selected source cannot be built.
    pub async fn compile<T: WasiView + 'static>(self) -> Result<Compiled<T>> {
        let config = self.config.or_else(|| env::var_os("OMNI_CONFIG").map(PathBuf::from));

        if let Some(config) = config {
            return create_from_manifest(&config).await;
        }

        let wasm = self.wasm.context(
            "no guest specified: pass a <wasm> path, or --config <omni.toml> (or set OMNI_CONFIG)",
        )?;

        let name = wasm.file_stem().and_then(|s| s.to_str()).unwrap_or("unknown");
        init_env(name)?;
        tracing::info!("initializing runtime");

        let (engine, linker, options) = engine_and_linker()?;

        let source = FileSource::new(wasm);
        let guests = source.load(&engine).await?;

        tracing::info!("runtime initialized");

        Ok(Compiled {
            engine,
            linker,
            options,
            guests,
            // The single-file shorthand carries no routes: its sole guest is the
            // catch-all for every trigger it can answer.
            routes: Routes::default(),
            // ...and no host-mediated links: one guest has nobody to dispatch to.
            link_interfaces: BTreeSet::new(),
        })
    }
}

/// Build a runtime from a deployment manifest (`omni.toml`).
///
/// Resolves every `[[guest]]` source, builds the shared engine + linker, records
/// the first guest as the default entry, and assembles the per-trigger route
/// tables from the `[[route.*]]` sections.
///
/// # Errors
///
/// Will fail if the manifest cannot be loaded, if a guest uses a source kind not
/// yet supported, or if a guest component cannot be loaded.
#[instrument]
pub async fn create_from_manifest<T: WasiView + 'static>(manifest: &Path) -> Result<Compiled<T>> {
    let parsed = Manifest::load(manifest)?;

    // The first guest entry doubles as the telemetry/component name for now.
    let component_name = parsed
        .guests
        .first()
        .map_or_else(|| GuestId::from("omnia"), |entry| GuestId::from(entry.id.as_str()));
    init_env(component_name.as_str())?;
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
            SourceSpec::Oci(_) => {
                bail!("guest `{id}`: OCI sources are not yet supported")
            }
        };
        guests.extend(loaded);
    }

    let routes = route_tables(&parsed.route);

    // Union the per-guest `link` allow-lists: the linker is shared, so an
    // interface dispatched for one guest is wired once for all (§4.4). The
    // floor keeps these as opaque interface strings.
    let link_interfaces: BTreeSet<Box<str>> = parsed
        .guests
        .iter()
        .flat_map(|entry| entry.link.iter())
        .map(|interface| Box::from(interface.as_str()))
        .collect();

    tracing::info!("runtime initialized from manifest");

    Ok(Compiled {
        engine,
        linker,
        options,
        guests,
        routes,
        link_interfaces,
    })
}

/// Convert the manifest's parsed routes into the registry's `GuestId`-typed,
/// per-trigger route tables.
fn route_tables(spec: &RouteSpec) -> Routes {
    let http = HttpRoutes::new(
        spec.http.iter().map(|e| (e.prefix.clone(), GuestId::from(e.guest.as_str()))),
    );
    let messaging = TopicRoutes::new(
        spec.messaging.iter().map(|e| (e.topic.clone(), GuestId::from(e.guest.as_str()))),
    );
    let websocket = TopicRoutes::new(
        spec.websocket.iter().map(|e| (e.topic.clone(), GuestId::from(e.guest.as_str()))),
    );
    Routes::new(http, messaging, websocket)
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
/// be [`link`]ed against host interfaces and [`build`]t into a [`Registry`].
///
/// [`link`]: Self::link
/// [`build`]: Self::build
pub struct Compiled<T: WasiView + 'static> {
    engine: Engine,
    linker: Linker<T>,
    options: RuntimeOptions,
    guests: Vec<LoadedGuest>,
    routes: Routes,
    /// Union of the per-guest `link` allow-lists — the host-mediated interfaces
    /// to polyfill onto the shared linker (empty for the single-file shorthand).
    link_interfaces: BTreeSet<Box<str>>,
}

impl<T: WasiView> Compiled<T> {
    /// Link a WASI host's interfaces into the shared Linker.
    ///
    /// Chainable: returns `&mut Self` so several hosts can be linked in turn.
    ///
    /// # Errors
    ///
    /// Will fail if the host cannot be added to the Linker.
    pub fn link<H: Host<T>>(&mut self) -> Result<&mut Self> {
        H::add_to_linker(&mut self.linker)?;
        Ok(self)
    }

    /// Pre-instantiate every loaded guest against the shared Linker and assemble
    /// the [`Registry`].
    ///
    /// Consumes the builder: pre-instantiation happens once, here, after all
    /// hosts are linked — so no host can be linked after the guests are frozen.
    /// Per call only a fresh instantiate on a new store remains.
    ///
    /// # Errors
    ///
    /// Returns an error if host-mediated imports cannot be polyfilled, a
    /// component cannot be pre-instantiated, or the registry cannot be assembled.
    pub fn build(self) -> Result<Registry<T>>
    where
        T: WrpcView,
    {
        // The selector strategy is fixed to the floor default; consumers project
        // their identity scheme onto the opaque `GuestId` it returns.
        let dispatch = DispatchHandle::new(
            Arc::new(FirstArgSelector),
            self.link_interfaces,
            self.options.max_dispatch_depth,
        );

        // Polyfill host-mediated imports onto the shared linker *before*
        // pre-instantiation: an import that is neither host-satisfied nor
        // allow-listed then fails fast at `instantiate_pre`. Consuming `self`
        // makes the linker ours to mutate — no defensive clone.
        let mut linker = self.linker;
        link_dynamic(&self.engine, &mut linker, &self.guests, &dispatch)?;

        let mut guests = HashMap::with_capacity(self.guests.len());
        for loaded in &self.guests {
            let instance_pre = linker
                .instantiate_pre(&loaded.component)
                .map_err(anyhow::Error::from)
                .with_context(|| format!("pre-instantiating guest `{}`", loaded.id))?;
            guests.insert(loaded.id.clone(), Guest::local(loaded.id.clone(), instance_pre));
        }

        Registry::new(self.engine, self.options, guests, self.routes, dispatch)
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
