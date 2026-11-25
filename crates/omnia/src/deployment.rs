//! # WebAssembly Initiator

mod manifest;

use std::env;
#[cfg(feature = "loader")]
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use anyhow::{Context, Result, bail};
pub use manifest::{
    Env, GuestEntry, GuestRoutes, Manifest, Mount, PluginsConfig, RegistryConfig, Transport,
    TransportKind,
};
use omnia_core::wasmtime::component::Linker;
use omnia_core::wasmtime::{Config, Engine};
use omnia_core::{
    Backends, GuestId, Host, LevelFilter, LinkSeam, LoadedGuest, MountRegistry, Registry,
    RegistryParts, Routes, Runtime, RuntimeOptions, RuntimeParts, Source, SourceSpec, Stdio,
    StoreCtx, Telemetry, telemetry,
};
#[cfg(feature = "loader")]
use omnia_core::{Grants, RegistrySource};
#[cfg(feature = "link")]
use omnia_link::InProcessLinks;
#[cfg(feature = "loader")]
use omnia_plugin::{FsStore, NoStore, Plugins, RegistryClient, WasiPlugins};

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
///     .build::<Backends>()
///     .await?;
/// ```
#[derive(Debug, Default)]
pub struct DeploymentBuilder {
    manifest: Option<Manifest>,
    args: Vec<String>,
    mode: Mode,
    level: Option<LevelFilter>,
    runtime_level: Option<LevelFilter>,
    allow_empty: bool,
    program_name: Option<String>,
    options: Option<RuntimeOptions>,
    guest_timeout: Option<Duration>,
    max_dispatch_depth: Option<usize>,
    stdio: Option<Stdio>,
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

    /// Set the command's arguments, everything after `--`.
    ///
    /// The command run receives them after the [program name](Self::program_name)
    /// as `argv[0]`. No other guest sees them. A [`Mode::Server`] deployment
    /// takes none, so [`build`](Self::build) refuses any.
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

    /// Filter the runtime's own crates ([`telemetry::RUNTIME`]) on the host console.
    ///
    /// Set, they step from a server default with the same verbosity flags while
    /// guests keep the run's [level](Self::level). Unset, they sit at the process
    /// `RUST_LOG`'s bare level as written, else at `warn`. A `RUST_LOG` directive
    /// naming one stands. The refinement is the console layer's alone
    /// ([`Telemetry::runtime_level`]): spans, exported events, and every guest
    /// follow the run's level.
    #[must_use]
    pub const fn runtime_level(mut self, level: LevelFilter) -> Self {
        self.runtime_level = Some(level);
        self
    }

    /// Mark the deployment as dynamically populated: the guest set may start
    /// empty and grow at run time via
    /// [`Runtime::register`](omnia_core::Runtime::register).
    ///
    /// This only relaxes the "at least one guest" check — static trigger
    /// routing (HTTP/messaging/websocket) is built at boot; registered
    /// guests are reachable via host-mediated link dispatch and host→guest
    /// [`Dispatcher::invoke`](omnia_core::Dispatcher::invoke).
    #[must_use]
    pub const fn dynamic(mut self) -> Self {
        self.allow_empty = true;
        self
    }

    /// Set the deployment's program name: the telemetry component name and,
    /// in command mode, the `argv[0]` prepended to the command's arguments.
    ///
    /// The `runtime!` macro sets the invoking crate's package name; unset,
    /// the name is `omnia`.
    #[must_use]
    pub fn program_name(mut self, name: impl Into<String>) -> Self {
        self.program_name = Some(name.into());
        self
    }

    /// Run the deployment under `options`, in place of the ones the
    /// environment sets.
    ///
    /// None of the variables [`RuntimeOptions`] maps is read then. To combine
    /// the two:
    /// - [`RuntimeOptions::with_env`] applies an operator's variables over
    ///   options built in code;
    /// - a field set on what [`RuntimeOptions::load_env`] returns overrides
    ///   the operator.
    ///
    /// [`guest_timeout`](Self::guest_timeout) and
    /// [`max_dispatch_depth`](Self::max_dispatch_depth) still apply on top.
    /// The result is [validated](RuntimeOptions::validate) before the engine
    /// and its instance pool are built. An artifact compiled ahead of time
    /// loads only under the [`compile_options`](RuntimeOptions::compile_options)
    /// it was compiled with.
    #[must_use]
    pub const fn options(mut self, options: RuntimeOptions) -> Self {
        self.options = Some(options);
        self
    }

    /// Override the wall-clock cap on server and server-rooted link-dispatch
    /// invocations for this deployment.
    ///
    /// Unset defers to the deployment's options: those set with
    /// [`options`](Self::options), else `GUEST_TIMEOUT_MS`.
    #[must_use]
    pub const fn guest_timeout(mut self, timeout: Duration) -> Self {
        self.guest_timeout = Some(timeout);
        self
    }

    /// Override the per-chain host-mediated dispatch depth bound for this
    /// deployment.
    ///
    /// Unset defers to the deployment's options: those set with
    /// [`options`](Self::options), else `MAX_DISPATCH_DEPTH`.
    #[must_use]
    pub const fn max_dispatch_depth(mut self, depth: usize) -> Self {
        self.max_dispatch_depth = Some(depth);
        self
    }

    /// Direct every guest's stdout and stderr to `stdio` instead of the
    /// process streams — how a host suite reads what a command wrote.
    #[must_use]
    pub fn stdio(mut self, stdio: Stdio) -> Self {
        self.stdio = Some(stdio);
        self
    }

    /// Resolve the manifest into a [`Deployment`] over the backend bundle `B`.
    ///
    /// If no manifest was supplied, the path in `OMNIA_MANIFEST` is loaded.
    /// Guests the process already holds as bytes load (and compile) here; a
    /// path or package source is declared and loads at its first use. A
    /// leading `~/` on a mount path or the package store expands against
    /// `$HOME` here.
    ///
    /// # Errors
    ///
    /// Returns an error if a [`Mode::Server`] deployment is given
    /// [arguments](Self::args), no manifest resolves, the manifest is invalid, the
    /// runtime options do not load or [validate](RuntimeOptions::validate), a
    /// `~/` path has no `$HOME` to expand against, the package store lies
    /// beneath a writable mount, or the deployment cannot be built.
    pub async fn build<B: Backends>(self) -> Result<Deployment<B>> {
        let program_name = self.program_name.unwrap_or_else(|| "omnia".to_owned());
        let args = argv(self.mode, &program_name, self.args)?;

        let mut manifest = if let Some(manifest) = self.manifest {
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
        manifest.expand_home()?;

        // read once, so a bad configuration fails startup rather than the first load
        #[cfg(feature = "loader")]
        let registry_config = manifest.registry_config()?;

        // an operator `COMPONENT` override wins over the program name
        let name = env::var("COMPONENT").unwrap_or(program_name);

        // the run's directives, decided once for every span, exported event,
        // and guest; the console alone shows the runtime's crates from a server's
        // default, and exporters follow OpenTelemetry's own endpoint resolution
        let process = env::var("RUST_LOG").ok();
        let rust_log = telemetry::directives(self.level, self.mode.level(), process.as_deref());
        let runtime = telemetry::bare(self.runtime_level, Mode::Server.level(), process.as_deref());
        Telemetry::new(&name)
            .filter(&rust_log)
            .runtime_level(runtime)
            .build()
            .context("initializing telemetry")?;
        tracing::debug!("initializing runtime");

        // the options in code, else the environment's, checked once the overrides apply
        let mut options = match self.options {
            Some(options) => options,
            None => RuntimeOptions::load_env()?,
        };
        if let Some(timeout) = self.guest_timeout {
            options.guest_timeout = timeout;
        }
        if let Some(depth) = self.max_dispatch_depth {
            options.max_dispatch_depth = depth;
        }
        options.validate()?;

        let (engine, linker) = engine_and_linker(&options)?;

        // open every preopen once, so a misconfigured mount fails at startup
        let mounts = Arc::new(MountRegistry::open(manifest.preopens())?);

        // W^X: a store a guest could write through a mount loads nothing
        #[cfg(feature = "loader")]
        let store = match manifest.store_root() {
            Some(root) => {
                if let Some(mount) = mounts.beneath_writable(root)? {
                    bail!(
                        "the package store {} lies beneath the writable mount `{}` at {}: a \
                         guest could rewrite what the deployment loads",
                        root.display(),
                        mount.name,
                        mount.host_path.display()
                    );
                }
                Some(root.to_path_buf())
            }
            None => None,
        };

        // embedded bytes load now, in parallel; a path or package loads at first use
        let (embedded, declared): (Vec<Source>, Vec<Source>) = manifest
            .sources()
            .into_iter()
            .partition(|source| matches!(source.spec(), SourceSpec::Bytes(_)));
        let guests =
            futures::future::try_join_all(embedded.iter().map(|source| source.load(&engine)))
                .await?;

        // host-mediated dispatch only with `link`
        let seam: Arc<dyn LinkSeam<B>> = cfg_select! {
            feature = "link" => Arc::new(InProcessLinks::new()),
            _ => Arc::new(omnia_core::NoLinks),
        };

        Ok(Deployment {
            name,
            engine,
            linker,
            options,
            guests,
            declared,
            routes: manifest.routes(),
            seam,
            mounts,
            args: Arc::new(args),
            mode: self.mode,
            allow_empty: self.allow_empty,
            command_guest: manifest.command_guest(),
            #[cfg(feature = "loader")]
            loader: Loader {
                store,
                registry: None,
                config: registry_config,
                grants: manifest.plugins.map_or_else(Grants::all, |plugins| plugins.grants),
            },
            rust_log,
            inherit: manifest.env.inherit,
            stdio: self.stdio,
        })
    }
}

// the command run's argv, the program name first; a server runs no command, so
// it refuses any argument
fn argv(mode: Mode, program_name: &str, args: Vec<String>) -> Result<Vec<String>> {
    if mode.is_command() {
        return Ok(std::iter::once(program_name.to_owned()).chain(args).collect());
    }
    if !args.is_empty() {
        bail!("a server runs no command, so it takes no arguments: {} given", args.len());
    }
    Ok(args)
}

// Build the shared engine and its WASI-linked linker.
fn engine_and_linker<B: Send + 'static>(
    options: &RuntimeOptions,
) -> Result<(Engine, Linker<StoreCtx<B>>)> {
    let engine = Engine::new(&Config::from(options))?;

    // register services with runtime's Linker
    let mut linker = Linker::new(&engine);
    omnia_core::wasmtime_wasi::p3::add_to_linker(&mut linker)?;

    Ok((engine, linker))
}

/// A compiled set of WebAssembly components with their shared Linker over the
/// backend bundle `B`, ready to be [`host`]ed against WASI interfaces and
/// [assembled](Self::assemble) into a [`Runtime`].
///
/// [`host`]: Self::host
pub struct Deployment<B: 'static> {
    name: String,
    engine: Engine,
    linker: Linker<StoreCtx<B>>,
    options: RuntimeOptions,
    guests: Vec<LoadedGuest>,
    // path and package guests, each loaded at first use
    declared: Vec<Source>,
    routes: Routes,
    // `NoLinks` unless the `link` feature is on
    seam: Arc<dyn LinkSeam<B>>,
    mounts: Arc<MountRegistry>,
    args: Arc<Vec<String>>,
    mode: Mode,
    allow_empty: bool,
    command_guest: Option<GuestId>,
    #[cfg(feature = "loader")]
    loader: Loader,
    // every store's `RUST_LOG`
    rust_log: String,
    // the host variables every store's environment holds
    inherit: Vec<String>,
    stdio: Option<Stdio>,
}

impl<B: Backends> Deployment<B> {
    /// Link a WASI host's interfaces into the shared Linker.
    ///
    /// # Errors
    ///
    /// Will fail if the host cannot be added to the Linker.
    pub fn host<H: Host<B>>(&mut self) -> Result<&mut Self> {
        H::add_to_linker(&mut self.linker)?;
        Ok(self)
    }

    /// Select where package sources are acquired — the manifest's
    /// `source.package` guests at their first use, and the packages a guest
    /// names through the loader — in place of the client the deployment's
    /// `plugins` table builds.
    ///
    /// Without this call, [`assemble`](Self::assemble) installs a
    /// [`RegistryClient`] over the deployment's package store (`plugins.store`,
    /// else none) routed by its `plugins.registries` configuration.
    /// Chainable.
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

    /// Borrow the argv the command run receives, `argv[0]` the program name.
    ///
    /// Empty for a [`Mode::Server`] deployment.
    #[must_use]
    pub fn args(&self) -> &[String] {
        &self.args
    }

    /// Assemble this deployment into a [`Runtime`]: the guest loader host
    /// joins the linked hosts, the registry pre-instantiates the boot guests
    /// and tables the declared ones, then the loader's grant installs.
    ///
    /// Assembly consumes the deployment, so every host is linked before a
    /// guest pre-instantiates. A declared guest pre-instantiates against the
    /// same linker when its first use loads it. With the `link` feature,
    /// every import a guest makes outside the runtime's own namespaces is
    /// relayed to the guest exporting it. Without it, such an import is
    /// unresolved and assembly fails.
    ///
    /// The loader host is linked here, beside WASI, whenever omnia is built
    /// with the `loader` feature; wasmtime wires it only into worlds that
    /// import `omnia:plugins/loader`. The grant it serves is the runtime's
    /// first-use seam for the guests the deployment declares and the
    /// deployment's read-only mounts as the roots a path load reads through;
    /// every package — declared or named by a guest — is acquired through the
    /// runtime's own client, the deployment's package store first and the
    /// registry its `plugins.registries` routes it to after, unless
    /// [`registry_source`](Self::registry_source) selected one. A guest
    /// loaded by path or package runs under `plugins.grants` within the
    /// grants of the guest that loaded it.
    ///
    /// # Errors
    ///
    /// Returns an error if the loader host cannot be linked, the registry
    /// cannot be assembled, the `registries` configuration does not parse, or
    /// a writable mount shares or nests a read-only mount's directory.
    #[expect(
        clippy::unused_async,
        clippy::unused_async_trait_impl,
        reason = "embedders and the `runtime!` expansion await assembly"
    )]
    pub async fn assemble(self, backends: B) -> Result<Runtime<B>> {
        let deployment = self;
        #[cfg(feature = "loader")]
        let (deployment, loader) = deployment.with_loader_host()?;
        #[cfg(feature = "loader")]
        let registry = loader.registry()?;

        // name what reaches everything, so an operator sees what grants would
        // narrow, warning once another guest declares some
        let boot = deployment.guests.iter().map(|guest| (&guest.id, &guest.grants));
        let first_use = deployment.declared.iter().map(|source| (source.id(), source.grants()));
        let guests = boot.chain(first_use);
        let granted = guests.clone().any(|(_, grants)| !grants.is_all());
        for (id, _) in guests.filter(|(_, grants)| grants.is_all()) {
            let reach = "guest declares no grants: it reaches every variable, mount, and \
                         interface of the deployment";
            if granted {
                tracing::warn!(guest = %id, "{reach}");
            } else {
                tracing::info!(guest = %id, "{reach}");
            }
        }
        #[cfg(feature = "loader")]
        if loader.grants.is_all() {
            tracing::info!(
                "the deployment declares no `[plugins.grants]`: a guest loaded at run time \
                 reaches whatever its loader does"
            );
        }

        let runtime = Runtime::from_parts(RuntimeParts {
            name: Arc::from(deployment.name.as_str()),
            args: deployment.args.to_vec(),
            mounts: Arc::clone(&deployment.mounts),
            packages: cfg_select! {
                feature = "loader" => Some(registry),
                _ => None,
            },
            rust_log: deployment.rust_log.clone(),
            inherit: deployment.inherit.clone(),
            stdio: deployment.stdio.clone(),
            command_guest: deployment.command_guest.clone(),
            backends,
            registry: Arc::new(deployment.into_registry().context("assembling registry")?),
        });

        #[cfg(feature = "loader")]
        Plugins::install(&runtime, loader.grants).context("installing the guest loader")?;
        Ok(runtime)
    }

    // Linked beside WASI so wasmtime wires it only into worlds that import
    // `omnia:plugins/loader`; `assemble` hands the registry to the runtime.
    #[cfg(feature = "loader")]
    fn with_loader_host(mut self) -> Result<(Self, Loader)> {
        self.host::<WasiPlugins>().context("linking the guest loader host")?;
        let loader = std::mem::take(&mut self.loader);
        Ok((self, loader))
    }

    fn into_registry(self) -> Result<Registry<B>> {
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

cfg_select! {
    feature = "loader" => {
        struct Loader {
            // the package store's root; `None` keeps nothing fetched
            store: Option<PathBuf>,
            // `None` installs a `RegistryClient` over the store and the deployment's `registries`
            registry: Option<Arc<dyn RegistrySource>>,
            // the `registries` configuration, one document despite the public plural key
            config: Option<String>,
            // what a guest loaded by path or package runs under, within its loader's
            grants: Grants,
        }

        impl Default for Loader {
            fn default() -> Self {
                Self {
                    store: None,
                    registry: None,
                    config: None,
                    grants: Grants::all(),
                }
            }
        }

        impl Loader {
            fn registry(&self) -> Result<Arc<dyn RegistrySource>> {
                if let Some(registry) = &self.registry {
                    return Ok(Arc::clone(registry));
                }
                let config = self.config.as_deref();
                Ok(match &self.store {
                    Some(root) => Arc::new(RegistryClient::from_toml(config, FsStore::open(root))?),
                    None => Arc::new(RegistryClient::from_toml(config, NoStore)?),
                })
            }
        }
    }
    _ => {}
}
