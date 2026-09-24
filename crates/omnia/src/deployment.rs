//! # WebAssembly Initiator

mod manifest;
mod source;

use std::env;
use std::sync::Arc;
use std::time::Duration;

use anyhow::{Context, Result};
pub use manifest::{
    GuestEntry, GuestRoutes, Manifest, Mount, RegistryConfig, SourceSpec, Transport, TransportKind,
};
#[cfg(feature = "link")]
use omnia_core::ChainPolicy;
#[cfg(not(feature = "link"))]
use omnia_core::NoLinks;
use omnia_core::wasmtime::component::Linker;
use omnia_core::wasmtime::{Config, Engine};
use omnia_core::wasmtime_wasi::WasiView;
use omnia_core::{
    GuestId, HasChain, Host, LevelFilter, LinkSeam, LoadedGuest, MountRegistry, Registry, Routes,
    Runtime, RuntimeOptions, RuntimeParts, Server, StoreCtx, Telemetry,
};
#[cfg(feature = "link")]
use omnia_link::{FirstArgSelector, GuestSelector, InProcessLinks};
#[cfg(feature = "loader")]
use omnia_plugin::{OnDemand, Plugins, RegistryClient, RegistrySource, WasiPlugins};

use crate::Mode;

/// Builds a [`Deployment`] from an optional programmatic [`Manifest`].
///
/// When no manifest is set, [`build`](Self::build) loads the path in
/// `OMNIA_MANIFEST`.
///
/// ```ignore
/// let deployment = DeploymentBuilder::new()
///     .manifest(Manifest::from_wasm(wasm))
///     .args(args)
///     .mode(mode)
///     .build::<StoreCtx>()
///     .await?;
/// ```
#[derive(Debug, Default)]
pub struct DeploymentBuilder {
    manifest: Option<Manifest>,
    args: Vec<String>,
    mode: Mode,
    level: Option<LevelFilter>,
    allow_empty: bool,
    program_name: Option<String>,
    guest_timeout: Option<Duration>,
    max_dispatch_depth: Option<usize>,
}

impl DeploymentBuilder {
    /// Start a new builder with no source selected.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Set the deployment manifest.
    #[must_use]
    pub fn manifest(mut self, manifest: impl Into<Option<Manifest>>) -> Self {
        self.manifest = manifest.into();
        self
    }

    /// Set CLI arguments forwarded to the guest (everything after `--`).
    #[must_use]
    pub fn args(mut self, args: impl Into<Vec<String>>) -> Self {
        self.args = args.into();
        self
    }

    /// Set the deployment drive mode.
    #[must_use]
    pub const fn mode(mut self, mode: Mode) -> Self {
        self.mode = mode;
        self
    }

    /// Select the tracing level for the whole process: the host console and
    /// every guest's `RUST_LOG` alike.
    ///
    /// A selected level replaces whatever `RUST_LOG` the process carries.
    /// Unset, a run keeps the process `RUST_LOG` and falls back to the
    /// [mode's level](Mode::level) when it sets none. The process environment
    /// is never written: guests read the level through their WASI
    /// environment, the host through its subscriber.
    #[must_use]
    pub const fn level(mut self, level: LevelFilter) -> Self {
        self.level = Some(level);
        self
    }

    /// Mark the deployment as dynamically populated: the guest set may start
    /// empty and grow at run time via
    /// [`Runtime::register`](omnia_core::Runtime::register).
    ///
    /// This only relaxes the "at least one guest" check — static trigger
    /// routing (HTTP/messaging/websocket/CLI) is built at boot; registered
    /// guests are reachable via host-mediated link dispatch and host→guest
    /// [`Dispatcher::invoke`](omnia_core::Dispatcher::invoke).
    #[must_use]
    pub const fn dynamic(mut self) -> Self {
        self.allow_empty = true;
        self
    }

    /// Set the deployment's program name: the telemetry component name and,
    /// in command mode, the `argv[0]` prepended to the guest's arguments.
    ///
    /// The `runtime!` macro sets the invoking crate's package name; unset,
    /// the name is `omnia`.
    #[must_use]
    pub fn program_name(mut self, name: impl Into<String>) -> Self {
        self.program_name = Some(name.into());
        self
    }

    /// Override the wall-clock cap on server and server-rooted link-dispatch
    /// invocations for this deployment. Unset defers to `GUEST_TIMEOUT_MS`.
    #[must_use]
    pub const fn guest_timeout(mut self, timeout: Duration) -> Self {
        self.guest_timeout = Some(timeout);
        self
    }

    /// Override the per-chain host-mediated dispatch depth bound for this
    /// deployment. Unset defers to `MAX_DISPATCH_DEPTH`.
    #[must_use]
    pub const fn max_dispatch_depth(mut self, depth: usize) -> Self {
        self.max_dispatch_depth = Some(depth);
        self
    }

    /// Resolve the manifest into a [`Deployment`].
    ///
    /// If no manifest was supplied, the path in `OMNIA_MANIFEST` is loaded.
    /// A guest is a raw wasm component or `omnia compile` output; either
    /// loads.
    ///
    /// # Errors
    ///
    /// Returns an error if no manifest resolves, the manifest is invalid, or
    /// the deployment cannot be built.
    pub async fn build<T: WasiView + 'static>(self) -> Result<Deployment<T>> {
        let manifest = if let Some(manifest) = self.manifest {
            manifest
        } else if self.allow_empty {
            // A dynamic deployment may start empty and register guests later.
            Manifest::new()
        } else {
            let path = env::var_os("OMNIA_MANIFEST")
                .context("no deployment manifest supplied and OMNIA_MANIFEST is unset")?;
            Manifest::load(path)?
        };
        manifest.validate(self.allow_empty)?;
        // Read once, here, so a missing or unreadable configuration fails
        // startup rather than the first package load.
        let registry_config = manifest.registry_config()?;
        #[cfg(feature = "loader")]
        let on_demand = manifest.on_demand();

        let program_name = self.program_name.unwrap_or_else(|| "omnia".to_owned());
        // The runtime-carried name read by telemetry, trigger servers, and
        // the bootstrap log. An operator `COMPONENT` override wins over the
        // program name — read once here, never written back to the process
        // environment.
        let name = env::var("COMPONENT").unwrap_or_else(|_| program_name.clone());

        let fallback = self.mode.level();
        init_telemetry(&name, self.level, fallback)?;
        tracing::debug!("initializing runtime");

        let (engine, linker, mut options) = engine_and_linker()?;
        if let Some(timeout) = self.guest_timeout {
            options.guest_timeout = timeout;
        }
        if let Some(depth) = self.max_dispatch_depth {
            options.max_dispatch_depth = depth;
        }

        // Open + identity-stamp every preopen once, here, so a misconfigured
        // mount fails fast at startup rather than per store.
        let mounts = Arc::new(MountRegistry::open(manifest.preopens())?);

        // Boot guests load (and compile) in parallel through the async
        // [`Source::load`] seam; order still follows the manifest.
        let sources = manifest.sources()?;
        let guests =
            futures::future::try_join_all(sources.iter().map(|source| source.load(&engine)))
                .await?;

        // In command mode the program name is prepended as `argv[0]`.
        let args = if self.mode.is_command() {
            std::iter::once(program_name).chain(self.args).collect()
        } else {
            self.args
        };

        Ok(Deployment {
            name,
            engine,
            linker,
            options,
            guests,
            routes: manifest.routes(),
            #[cfg(feature = "link")]
            selector: Arc::new(FirstArgSelector),
            mounts,
            args: Arc::new(args),
            mode: self.mode,
            allow_empty: self.allow_empty,
            command_guest: manifest.command_guest(),
            registry_config,
            #[cfg(feature = "loader")]
            loader: Loader {
                on_demand,
                registry: None,
            },
            level: self.level,
            fallback,
        })
    }
}

/// A compiled set of WebAssembly components with their shared Linker, ready to
/// be [`host`]ed against WASI interfaces and assembled into a [`Registry`].
///
/// [`host`]: Self::host
pub struct Deployment<T: WasiView + 'static> {
    // Deployment name carried onto the runtime for trigger servers and the
    // bootstrap log (the program name, unless `build` honored an operator
    // `COMPONENT` override).
    name: String,
    engine: Engine,
    linker: Linker<T>,
    options: RuntimeOptions,
    guests: Vec<LoadedGuest>,
    routes: Routes,
    // Host-mediated dispatch selector.
    #[cfg(feature = "link")]
    selector: Arc<dyn GuestSelector>,
    // Mount registry opened from the manifest's resolved preopens.
    mounts: Arc<MountRegistry>,
    // Guest argv threaded into every store. Empty for long-lived servers; in
    // command mode the deployment name is prepended as `argv[0]`.
    args: Arc<Vec<String>>,
    // Whether this deployment runs a one-shot `wasi:cli` command.
    mode: Mode,
    // Whether the guest set may start empty and grow at run time.
    allow_empty: bool,
    // Command-mode guest identity derived from the manifest's marked entry.
    command_guest: Option<GuestId>,
    // The manifest's wasm-pkg configuration, resolved to one document despite
    // the public plural key, carried onto the runtime for the guest loader.
    registry_config: Option<String>,
    // What `assemble` installs on the guest loader.
    #[cfg(feature = "loader")]
    loader: Loader,
    // The selected tracing level and the mode's fallback, carried onto the
    // runtime to set `RUST_LOG` in every store it builds.
    level: Option<LevelFilter>,
    fallback: LevelFilter,
}

#[cfg(feature = "loader")]
#[derive(Default)]
struct Loader {
    // The manifest's on-demand guests.
    on_demand: Vec<(GuestId, OnDemand)>,
    // The embedder's registry source; `None` installs a cacheless
    // `RegistryClient` over the deployment's `registries` configuration.
    registry: Option<Arc<dyn RegistrySource>>,
}

/// Store bound every deployment store context satisfies; kept as a named bound
/// for source compatibility with embedders that spell it.
pub trait LinkStore: WasiView + HasChain + 'static {}

impl<T: WasiView + HasChain + 'static> LinkStore for T {}

impl<T: WasiView> Deployment<T> {
    /// Link a WASI host's interfaces into the shared Linker.
    ///
    /// # Errors
    ///
    /// Will fail if the host cannot be added to the Linker.
    pub fn host<H, B>(&mut self) -> Result<&mut Self>
    where
        H: Host<T> + Server<B>,
    {
        H::add_to_linker(&mut self.linker)?;
        Ok(self)
    }

    /// Override the host-mediated dispatch [`GuestSelector`].
    ///
    /// Defaults to [`FirstArgSelector`] — the runtime core's "first call argument is the
    /// identity" strategy. Chainable.
    #[cfg(feature = "link")]
    pub fn selector(&mut self, selector: impl GuestSelector) -> &mut Self {
        self.selector = Arc::new(selector);
        self
    }

    /// Select the registry the guest loader fetches on-demand package sources
    /// from — typically a [`RegistryClient`] with a cache store attached.
    ///
    /// Without this call, [`assemble`](Self::assemble) installs a cacheless
    /// [`RegistryClient`] routed by the deployment's `registries`
    /// configuration. Chainable.
    #[cfg(feature = "loader")]
    pub fn registry_source(&mut self, registry: impl RegistrySource) -> &mut Self {
        self.loader.registry = Some(Arc::new(registry));
        self
    }

    /// The deployment name carried onto the runtime for trigger servers and
    /// the bootstrap log.
    #[must_use]
    pub fn name(&self) -> &str {
        &self.name
    }

    /// The mount registry built from the deployment's preopens.
    #[must_use]
    pub fn mounts(&self) -> Arc<MountRegistry> {
        Arc::clone(&self.mounts)
    }

    /// Deployment drive mode.
    #[must_use]
    pub const fn mode(&self) -> Mode {
        self.mode
    }

    /// Borrow the guest argv.
    #[must_use]
    pub fn args(&self) -> &[String] {
        &self.args
    }

    /// The tracing level selected for this run, if any.
    #[must_use]
    pub const fn level(&self) -> Option<LevelFilter> {
        self.level
    }

    /// Assemble the guest [`Registry`].
    ///
    /// Consumes the deployment: pre-instantiation happens once, here, after all
    /// hosts are linked — so no host can be linked after the guests are frozen.
    /// Per call only a fresh instantiate on a new store remains. With the
    /// `link` feature, every import a guest makes outside the runtime's own
    /// namespaces is relayed to the guest exporting it; without it, such an
    /// import is unresolved and pre-instantiation fails.
    ///
    /// # Errors
    ///
    /// Returns an error if a relayed import cannot be polyfilled, a component
    /// cannot be pre-instantiated, or the registry cannot be assembled.
    pub fn into_registry(self) -> Result<Registry<T>>
    where
        T: LinkStore,
    {
        #[cfg(feature = "link")]
        let seam: Arc<dyn LinkSeam<T>> =
            Arc::new(InProcessLinks::new(self.selector, ChainPolicy::from(&self.options)));
        #[cfg(not(feature = "link"))]
        let seam: Arc<dyn LinkSeam<T>> = Arc::new(NoLinks);

        Registry::assemble(
            self.engine,
            self.linker,
            self.options,
            self.guests,
            self.routes,
            seam,
            self.allow_empty,
        )
    }
}

impl<B: Clone + Send + Sync + 'static> Deployment<StoreCtx<B>> {
    /// Assemble this deployment into a [`Runtime`]: the guest loader host
    /// joins the linked hosts, the registry pre-instantiates the boot guests,
    /// the loader's on-demand table installs, then the serve side of every
    /// guest's linked exports is wired.
    ///
    /// The loader host is linked here, beside WASI, whenever omnia is built
    /// with the `loader` feature; wasmtime wires it only into worlds that
    /// import `omnia:plugins/loader`. The table it serves is the manifest's
    /// `on_demand` guests; their package sources are fetched through the
    /// deployment's `registries` configuration unless
    /// [`registry_source`](Self::registry_source) selected a registry.
    ///
    /// # Errors
    ///
    /// Returns an error if the loader host cannot be linked, the registry
    /// cannot be assembled, the `registries` configuration does not parse, or
    /// a guest's linked exports cannot be served.
    pub async fn assemble(
        #[cfg_attr(
            not(feature = "loader"),
            expect(unused_mut, reason = "linking the loader host is the one mutation")
        )]
        mut self,
        backends: B,
    ) -> Result<Runtime<B>> {
        #[cfg(feature = "loader")]
        self.host::<WasiPlugins, B>().context("linking the guest loader host")?;
        #[cfg(feature = "loader")]
        let Loader { on_demand, registry } = std::mem::take(&mut self.loader);
        #[cfg(feature = "loader")]
        let registry = match registry {
            Some(registry) => registry,
            None => Arc::new(match &self.registry_config {
                Some(config) => RegistryClient::from_toml(config)?,
                None => RegistryClient::default(),
            }),
        };

        let runtime = Runtime::from_parts(RuntimeParts {
            name: Arc::from(self.name.as_str()),
            args: self.args.to_vec(),
            mounts: Arc::clone(&self.mounts),
            registry_config: self.registry_config.clone(),
            level: self.level,
            fallback: self.fallback,
            command_guest: self.command_guest.clone(),
            backends,
            registry: Arc::new(self.into_registry().context("assembling registry")?),
        });

        #[cfg(feature = "loader")]
        Plugins::install(&runtime, on_demand, registry).context("installing the guest loader")?;

        runtime.serve_links().await.context("serving the guests' linked exports")?;
        Ok(runtime)
    }
}

// Build the shared engine, WASI-linked linker, and runtime options.
fn engine_and_linker<T: WasiView + 'static>() -> Result<(Engine, Linker<T>, RuntimeOptions)> {
    let options = RuntimeOptions::load_env()?;
    let engine = Engine::new(&Config::from(&options))?;

    // register services with runtime's Linker
    let mut linker = Linker::new(&engine);
    omnia_core::wasmtime_wasi::p2::add_to_linker_async(&mut linker)?;
    omnia_core::wasmtime_wasi::p3::add_to_linker(&mut linker)?;

    Ok((engine, linker, options))
}

// Initialize telemetry for the runtime at the run's level: a selected level
// is the console filter outright; otherwise the process `RUST_LOG` stands
// and `fallback` fills its absence.
//
// Telemetry initialization is idempotent (`Telemetry::build`): the first call
// in the process — here or in an embedder — installs the subscriber and
// providers, and later deployments reuse them.
fn init_telemetry(name: &str, level: Option<LevelFilter>, fallback: LevelFilter) -> Result<()> {
    let mut builder = Telemetry::new(name).fallback(fallback);
    if let Some(level) = level {
        builder = builder.filter(level.to_string());
    }
    if let Ok(endpoint) = env::var("OTEL_GRPC_URL") {
        builder = builder.endpoint(endpoint);
    } else {
        tracing::debug!("OTEL_GRPC_URL unset; using OpenTelemetry defaults");
    }
    builder.build().context("initializing telemetry")
}

#[cfg(test)]
mod tests {
    use omnia_core::RuntimeOptions;
    use omnia_core::wasmtime::{Config, Engine};

    #[test]
    fn builds_pooling() {
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
            pool_decommit_batch_size: 8,
            ..RuntimeOptions::load_env().expect("should load")
        };
        Engine::new(&Config::from(&options))
            .expect("decoupled multi-memory pooling config should build an engine");
    }

    #[test]
    fn builds_no_pooling() {
        let options = RuntimeOptions {
            pooling: false,
            ..RuntimeOptions::load_env().expect("should load")
        };
        Engine::new(&Config::from(&options)).expect("non-pooling config should build an engine");
    }
}
