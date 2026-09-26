//! # WebAssembly Initiator

mod manifest;

use std::env;
use std::sync::Arc;
use std::time::Duration;

use anyhow::{Context, Result};
pub use manifest::{
    GuestEntry, GuestRoutes, Manifest, Mount, RegistryConfig, Transport, TransportKind,
};
#[cfg(feature = "link")]
use omnia_core::ChainPolicy;
#[cfg(feature = "loader")]
use omnia_core::RegistrySource;
use omnia_core::wasmtime::component::Linker;
use omnia_core::wasmtime::{Config, Engine};
use omnia_core::wasmtime_wasi::WasiView;
use omnia_core::{
    GuestId, HasChain, HasDispatcher, Host, LevelFilter, LinkSeam, LoadedGuest, MountRegistry,
    NoLinks, Registry, RegistryParts, Routes, Runtime, RuntimeOptions, RuntimeParts, Server,
    Source, SourceSpec, StoreCtx, Telemetry, telemetry,
};
#[cfg(feature = "link")]
use omnia_link::{FirstArgSelector, GuestSelector, InProcessLinks};
#[cfg(feature = "loader")]
use omnia_plugin::{Plugins, RegistryClient, WasiPlugins};

use crate::Mode;

/// Builds a [`Deployment`] from an optional programmatic [`Manifest`].
///
/// When no manifest is set, [`build`](Self::build) loads the path in
/// `OMNIA_MANIFEST`.
///
/// ```ignore
/// let deployment = DeploymentBuilder::new()
///     .manifest(Manifest::from_wasm(wasm)?)
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
    /// The level composes with the process `RUST_LOG`
    /// ([`telemetry::directives`]): it displaces the variable's bare level,
    /// and the variable's targeted directives (`tower=off`, `my_sdk=debug`)
    /// stay in force on top. Unset, a run keeps the process `RUST_LOG` and
    /// falls back to the [mode's level](Mode::level) when it sets none. The
    /// process environment is never written: guests read the directives
    /// through their WASI environment, the host through its subscriber.
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
    /// Guests the process already holds as bytes load (and compile) here; a
    /// path or package source is declared and loads at its first use.
    ///
    /// # Errors
    ///
    /// Returns an error if no manifest resolves, the manifest is invalid, or
    /// the deployment cannot be built.
    pub async fn build<T: LinkStore>(self) -> Result<Deployment<T>> {
        let manifest = if let Some(manifest) = self.manifest {
            manifest
        } else if self.allow_empty {
            // a dynamic deployment may start empty and register guests later
            Manifest::new()
        } else {
            let path = env::var_os("OMNIA_MANIFEST")
                .context("no deployment manifest supplied and OMNIA_MANIFEST is unset")?;
            Manifest::load(path)?
        };
        manifest.validate(self.allow_empty, manifest::Features::COMPILED)?;

        // read once, so a bad configuration fails startup rather than the first load
        let registry_config = manifest.registry_config()?;

        // an operator `COMPONENT` override wins over the program name
        let program_name = self.program_name.unwrap_or_else(|| "omnia".to_owned());
        let name = env::var("COMPONENT").unwrap_or_else(|_| program_name.clone());

        // the run's directives, decided once for the host console and every guest
        let rust_log = telemetry::directives(
            self.level,
            self.mode.level(),
            env::var("RUST_LOG").ok().as_deref(),
        );
        init_telemetry(&name, &rust_log)?;
        tracing::debug!("initializing runtime");

        let (engine, linker, mut options) = engine_and_linker()?;
        if let Some(timeout) = self.guest_timeout {
            options.guest_timeout = timeout;
        }
        if let Some(depth) = self.max_dispatch_depth {
            options.max_dispatch_depth = depth;
        }

        // open every preopen once, so a misconfigured mount fails at startup
        let mounts = Arc::new(MountRegistry::open(manifest.preopens())?);

        // embedded bytes load now, in parallel; a path or package loads at first use
        let (embedded, declared): (Vec<Source>, Vec<Source>) = manifest
            .sources()
            .into_iter()
            .partition(|source| matches!(source.spec(), SourceSpec::Bytes(_)));
        let guests =
            futures::future::try_join_all(embedded.iter().map(|source| source.load(&engine)))
                .await?;

        // command mode prepends the program name as `argv[0]`
        let args = if self.mode.is_command() {
            std::iter::once(program_name).chain(self.args).collect()
        } else {
            self.args
        };

        // no host-mediated dispatch unless `link` overrides the seam below
        let deployment = Deployment {
            name,
            engine,
            linker,
            options,
            guests,
            declared,
            routes: manifest.routes(),
            seam: Arc::new(NoLinks),
            mounts,
            args: Arc::new(args),
            mode: self.mode,
            allow_empty: self.allow_empty,
            command_guest: manifest.command_guest(),
            registry_config,
            #[cfg(feature = "loader")]
            loader: Loader { registry: None },
            rust_log,
        };
        #[cfg(feature = "link")]
        let deployment = Deployment {
            seam: Arc::new(InProcessLinks::new(
                Arc::new(FirstArgSelector),
                ChainPolicy::from(&deployment.options),
            )),
            ..deployment
        };
        Ok(deployment)
    }
}

/// A compiled set of WebAssembly components with their shared Linker, ready to
/// be [`host`]ed against WASI interfaces and assembled into a [`Registry`].
///
/// [`host`]: Self::host
pub struct Deployment<T: WasiView + 'static> {
    name: String,
    engine: Engine,
    linker: Linker<T>,
    options: RuntimeOptions,
    guests: Vec<LoadedGuest>,
    // path and package guests, each loaded at first use
    declared: Vec<Source>,
    routes: Routes,
    // `NoLinks` unless the `link` feature is on
    seam: Arc<dyn LinkSeam<T>>,
    mounts: Arc<MountRegistry>,
    args: Arc<Vec<String>>,
    mode: Mode,
    allow_empty: bool,
    command_guest: Option<GuestId>,
    // one document despite the public plural key
    registry_config: Option<String>,
    #[cfg(feature = "loader")]
    loader: Loader,
    // every store's `RUST_LOG`
    rust_log: String,
}

#[cfg(feature = "loader")]
#[derive(Default)]
struct Loader {
    // `None` installs a cacheless `RegistryClient` over the deployment's `registries`
    registry: Option<Arc<dyn RegistrySource>>,
}

#[cfg(feature = "loader")]
impl Loader {
    fn registry(&self, config: Option<&str>) -> Result<Arc<dyn RegistrySource>> {
        Ok(match &self.registry {
            Some(registry) => Arc::clone(registry),
            None => Arc::new(match config {
                Some(config) => RegistryClient::from_toml(config)?,
                None => RegistryClient::default(),
            }),
        })
    }
}

/// Store bound [`DeploymentBuilder::build`] requires; every deployment store
/// context ([`StoreCtx`]) satisfies it.
pub trait LinkStore: WasiView + HasChain + HasDispatcher + 'static {}

impl<T: WasiView + HasChain + HasDispatcher + 'static> LinkStore for T {}

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
    pub fn selector(&mut self, selector: impl GuestSelector) -> &mut Self
    where
        T: HasChain + HasDispatcher,
    {
        // the seam holds only selector and policy until assembly, so rebuilding loses nothing
        self.seam =
            Arc::new(InProcessLinks::new(Arc::new(selector), ChainPolicy::from(&self.options)));
        self
    }

    /// Select the registry package sources are fetched from — the manifest's
    /// `source.package` guests at their first use, and the packages a guest
    /// names through the loader — typically a [`RegistryClient`] with a cache
    /// store attached.
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

    /// Assemble the guest [`Registry`].
    ///
    /// Consumes the deployment: pre-instantiation of the boot guests happens
    /// once, here, after all hosts are linked — so no host can be linked
    /// after the guests are frozen; a declared guest pre-instantiates against
    /// the same linker when its first use loads it. Per call only a fresh
    /// instantiate on a new store remains. With the `link` feature, every
    /// import a guest makes outside the runtime's own namespaces is relayed
    /// to the guest exporting it; without it, such an import is unresolved
    /// and pre-instantiation fails.
    ///
    /// # Errors
    ///
    /// Returns an error if a relayed import cannot be polyfilled, a component
    /// cannot be pre-instantiated, or the registry cannot be assembled.
    pub fn into_registry(self) -> Result<Registry<T>> {
        Registry::assemble(RegistryParts {
            engine: self.engine,
            linker: self.linker,
            options: self.options,
            loaded: self.guests,
            declared: self.declared,
            routes: self.routes,
            seam: self.seam,
            allow_empty: self.allow_empty,
        })
    }
}

impl<B: Clone + Send + Sync + 'static> Deployment<StoreCtx<B>> {
    /// Assemble this deployment into a [`Runtime`]: the guest loader host
    /// joins the linked hosts, the registry pre-instantiates the boot guests
    /// and tables the declared ones, the loader's grant installs, then the
    /// serve side of every boot guest's linked exports is wired.
    ///
    /// The loader host is linked here, beside WASI, whenever omnia is built
    /// with the `loader` feature; wasmtime wires it only into worlds that
    /// import `omnia:plugins/loader`. The grant it serves is the runtime's
    /// first-use seam for the guests the deployment declares, the
    /// deployment's read-only mounts as the roots a path load reads through,
    /// and the `registries` configuration every package — declared or named
    /// by a guest — is fetched by unless
    /// [`registry_source`](Self::registry_source) selected a registry.
    ///
    /// # Errors
    ///
    /// Returns an error if the loader host cannot be linked, the registry
    /// cannot be assembled, the `registries` configuration does not parse, a
    /// writable mount shares or nests a read-only mount's directory, or a
    /// guest's linked exports cannot be served.
    pub async fn assemble(self, backends: B) -> Result<Runtime<B>> {
        let deployment = self;
        #[cfg(feature = "loader")]
        let (deployment, loader) = deployment.with_loader_host()?;
        #[cfg(feature = "loader")]
        let registry = loader.registry(deployment.registry_config.as_deref())?;

        let runtime = Runtime::from_parts(RuntimeParts {
            name: Arc::from(deployment.name.as_str()),
            args: deployment.args.to_vec(),
            mounts: Arc::clone(&deployment.mounts),
            #[cfg(feature = "loader")]
            packages: Some(Arc::clone(&registry)),
            #[cfg(not(feature = "loader"))]
            packages: None,
            registry_config: deployment.registry_config.clone(),
            rust_log: deployment.rust_log.clone(),
            command_guest: deployment.command_guest.clone(),
            backends,
            registry: Arc::new(deployment.into_registry().context("assembling registry")?),
        });

        #[cfg(feature = "loader")]
        Plugins::install(&runtime, registry).context("installing the guest loader")?;

        runtime.serve_links().await.context("serving the guests' linked exports")?;
        Ok(runtime)
    }

    // Linked beside WASI so wasmtime wires it only into worlds that import
    // `omnia:plugins/loader`; `assemble` installs the registry on it.
    #[cfg(feature = "loader")]
    fn with_loader_host(mut self) -> Result<(Self, Loader)> {
        self.host::<WasiPlugins, B>().context("linking the guest loader host")?;
        let loader = std::mem::take(&mut self.loader);
        Ok((self, loader))
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

// exporters follow OpenTelemetry's own endpoint resolution; none without an endpoint
fn init_telemetry(name: &str, rust_log: &str) -> Result<()> {
    Telemetry::new(name).filter(rust_log).build().context("initializing telemetry")
}

#[cfg(test)]
mod tests {
    use omnia_core::RuntimeOptions;
    use omnia_core::wasmtime::{Config, Engine};

    #[test]
    fn builds_pooling() {
        // sized small so the reservation stays cheap
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
