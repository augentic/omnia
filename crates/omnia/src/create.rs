//! # WebAssembly Initiator

use std::collections::{BTreeMap, BTreeSet};
use std::env;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use anyhow::{Context, Result};
use wasmtime::component::Linker;
use wasmtime::{Config, Engine};
use wasmtime_wasi::WasiView;
use wrpc_wasmtime::WrpcView;

use crate::dispatch::{self, DispatchHandle};
use crate::manifest::Manifest;
use crate::registry::{Guest, Registry};
use crate::routing::Routes;
use crate::selector::{FirstArgSelector, GuestSelector};
use crate::source::{LoadedGuest, Source};
use crate::working_tree::{ResolvedPreopen, WorkingTreeRegistry};
use crate::{Host, RuntimeOptions, Telemetry};

/// Selects where a runtime's guests come from, then [`compile`]s them into a
/// [`Compiled`] runtime ready for host linking.
///
/// The single-file shorthand ([`wasm`]) and the manifest-driven deployment
/// ([`config`]) are both expressed here; [`compile`] resolves whichever is set —
/// falling back to the `OMNIA_CONFIG` environment variable for the manifest.
///
/// ```ignore
/// let compiled = RegistryBuilder::new()
///     .wasm(wasm)
///     .config(config)
///     .args(args)
///     .command(command)
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
    args: Vec<String>,
    command: bool,
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

    /// Set the deployment manifest (`omnia.toml`) path for a multi-guest
    /// deployment.
    #[must_use]
    pub fn config(mut self, config: impl Into<Option<PathBuf>>) -> Self {
        self.config = config.into();
        self
    }

    /// Set CLI arguments forwarded to the guest (everything after `--`).
    #[must_use]
    pub fn args(mut self, args: impl Into<Vec<String>>) -> Self {
        self.args = args.into();
        self
    }

    /// Select command mode: prepend the deployment name as `argv[0]` for
    /// `wasi:cli` guests. Long-lived server deployments leave argv empty.
    #[must_use]
    pub const fn command(mut self, command: bool) -> Self {
        self.command = command;
        self
    }

    /// Resolve the configured source into a [`Compiled`] runtime, choosing
    /// single-file or manifest-driven population.
    ///
    /// Resolution: a `config` path (set via [`config`](Self::config) or the
    /// `OMNIA_CONFIG` environment variable) selects a manifest-driven deployment;
    /// otherwise the `wasm` path is the one-guest shorthand. At least one of the
    /// two must be provided.
    ///
    /// # Errors
    ///
    /// Returns an error if neither a config nor a wasm path is available, or if
    /// the selected source cannot be built.
    pub async fn compile<T: WasiView + 'static>(self) -> Result<Compiled<T>> {
        let manifest = self.config.or_else(|| env::var_os("OMNIA_CONFIG").map(PathBuf::from));

        let plan = if let Some(manifest) = manifest {
            let parsed = Manifest::load(&manifest)?;
            let base = manifest.parent().unwrap_or_else(|| Path::new("."));

            Plan {
                name: parsed.name().to_owned(),
                sources: parsed.sources(base)?,
                routes: parsed.routes(),
                links: parsed.links(),
                preopens: working_tree(parsed.mounts(base)),
                args: self.args,
                command: self.command,
            }
        } else {
            let wasm = self.wasm.context(
                "no guest specified: pass a <wasm> path, or --config <omnia.toml> (or set OMNIA_CONFIG)",
            )?;

            // The single-file shorthand is a one-guest deployment
            let source = Source::new(wasm);

            Plan {
                name: source.id().as_str().to_owned(),
                sources: vec![source],
                routes: Routes::default(),
                links: BTreeSet::new(),
                preopens: working_tree(Vec::new()),
                args: self.args,
                command: self.command,
            }
        };

        init_env(&plan.name)?;
        tracing::info!("initializing runtime");

        Compiled::from_plan(plan).await
    }
}

// add root preopen if OMNIA_WORKING_TREE is set
fn working_tree(mut preopens: Vec<ResolvedPreopen>) -> Vec<ResolvedPreopen> {
    if let Some(path) = env::var_os("OMNIA_WORKING_TREE")
        && !preopens.iter().any(|po| po.name == ".")
    {
        preopens.push(ResolvedPreopen::new(".".to_owned(), PathBuf::from(path), false));
    }
    preopens
}

/// Resolved deployment inputs shared by the manifest and single-file paths.
struct Plan {
    name: String,
    sources: Vec<Source>,
    routes: Routes,
    links: BTreeSet<Box<str>>,
    preopens: Vec<ResolvedPreopen>,
    args: Vec<String>,
    command: bool,
}

impl<T: WasiView + 'static> Compiled<T> {
    /// Acquire every guest named in `plan` through its [`Source`] and pair them
    /// with the shared engine and WASI-linked linker.
    ///
    /// Acquisition runs through the async [`Source::load`] seam so a future
    /// source kind (an OCI pull) slots in without a parallel loading path.
    async fn from_plan(plan: Plan) -> Result<Self> {
        let (engine, linker, options) = engine_and_linker()?;

        // Open + identity-stamp every preopen once, here, so a misconfigured
        // mount fails fast at startup rather than per store (RFC-55).
        let working_trees = Arc::new(WorkingTreeRegistry::open(plan.preopens)?);

        let mut guests = Vec::with_capacity(plan.sources.len());
        for source in &plan.sources {
            guests.extend(source.load(&engine).await?);
        }

        let args = if plan.command {
            std::iter::once(plan.name.clone()).chain(plan.args).collect()
        } else {
            plan.args
        };

        Ok(Self {
            engine,
            linker,
            options,
            guests,
            routes: plan.routes,
            links: plan.links,
            selector: Arc::new(FirstArgSelector),
            working_trees,
            args: Arc::new(args),
            command: plan.command,
        })
    }
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
/// be [`host`]ed against WASI interfaces and [`build`]t into a [`Registry`].
///
/// [`host`]: Self::host
/// [`build`]: Self::build
pub struct Compiled<T: WasiView + 'static> {
    engine: Engine,
    linker: Linker<T>,
    options: RuntimeOptions,
    guests: Vec<LoadedGuest>,
    routes: Routes,
    // Guest links — the host-mediated interfaces.
    links: BTreeSet<Box<str>>,
    // Host-mediated dispatch selector.
    selector: Arc<dyn GuestSelector>,
    // Working-tree registry from resolved preopens in [`from_plan`](Self::from_plan).
    working_trees: Arc<WorkingTreeRegistry>,
    // Guest argv threaded into every store. Empty for long-lived servers; in
    // command mode the deployment name is prepended as `argv[0]`.
    args: Arc<Vec<String>>,
    // Whether this deployment runs a one-shot `wasi:cli` command.
    command: bool,
}

use crate::Runtime;
use crate::Server;

impl<T: WasiView> Compiled<T> {
    /// Link a WASI host's interfaces into the shared Linker.
    ///
    /// Chainable: returns `&mut Self` so several hosts can be linked in turn.
    ///
    /// # Errors
    ///
    /// Will fail if the host cannot be added to the Linker.
    pub fn host<H, R>(&mut self) -> Result<&mut Self>
    where
        H: Host<T> + Server<R>,
        R: Runtime,
    {
        if !self.command || !<H as Server<R>>::IS_SERVER {
            H::add_to_linker(&mut self.linker)?;
        }
        Ok(self)
    }

    /// Override the host-mediated dispatch [`GuestSelector`].
    ///
    /// Defaults to [`FirstArgSelector`] — the floor's "first call argument is the
    /// identity" strategy. Chainable.
    pub fn selector(&mut self, selector: impl GuestSelector) -> &mut Self {
        self.selector = Arc::new(selector);
        self
    }

    /// The working-tree registry built from the deployment's preopens.
    #[must_use]
    pub fn working_trees(&self) -> Arc<WorkingTreeRegistry> {
        Arc::clone(&self.working_trees)
    }

    /// Whether this deployment drives a one-shot `wasi:cli` command.
    #[must_use]
    pub const fn command(&self) -> bool {
        self.command
    }

    /// Shared guest argv for threading into [`Runtime::store`](crate::Runtime).
    #[must_use]
    pub fn args(&self) -> &[String] {
        &self.args
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
        // The selector defaults to `FirstArgSelector` but may be overridden via
        // `selector`; consumers project their identity scheme onto the opaque
        // `GuestId` it returns.
        let dispatch =
            DispatchHandle::new(self.selector, self.links, self.options.max_dispatch_depth);

        // Polyfill host-mediated imports onto the shared linker *before*
        // pre-instantiation: an import that is neither host-satisfied nor
        // allow-listed then fails fast at `instantiate_pre`. Consuming `self`
        // makes the linker ours to mutate — no defensive clone.
        let mut linker = self.linker;
        dispatch::link(&self.engine, &mut linker, &self.guests, &dispatch)?;

        let mut guests = BTreeMap::new();
        for loaded in &self.guests {
            let instance_pre = linker
                .instantiate_pre(&loaded.component)
                .map_err(anyhow::Error::from)
                .with_context(|| format!("pre-instantiating guest `{}`", loaded.id))?;
            guests.insert(loaded.id.clone(), Guest::local(loaded.id.clone(), instance_pre));
        }

        tracing::info!(guests = guests.len(), "runtime initialized");

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
