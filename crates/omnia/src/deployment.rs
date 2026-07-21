//! # WebAssembly Initiator

mod manifest;
mod source;

use std::collections::BTreeSet;
use std::env;
use std::marker::PhantomData;
use std::sync::Arc;

use anyhow::{Context, Result};
pub use manifest::{
    GuestEntry, HttpRoute, Manifest, Mount, RouteSpec, SourceSpec, TopicRoute, Transport,
    TransportKind,
};
use source::ArtifactPolicy;
pub use source::{GuestArtifact, LoadedGuest, Source};
use wasmtime::component::Linker;
use wasmtime::{Config, Engine};
use wasmtime_wasi::WasiView;
use wrpc_wasmtime::WrpcView;

use crate::dispatch::{
    DispatchHandle, FirstArgSelector, GuestResolver, GuestSelector, HttpFallback,
};
use crate::mount::{MountRegistry, ResolvedPreopen};
use crate::registry::{GuestId, Registry, Routes};
use crate::{Host, Mode, RuntimeOptions, Server, Telemetry};

/// Typestate for [`DeploymentBuilder`]: only raw `.wasm` sources load (the
/// safe default).
#[derive(Debug)]
pub struct WasmOnly;

/// Typestate for [`DeploymentBuilder`]: pre-compiled `.bin` sources are
/// admitted, so [`build`](DeploymentBuilder::build) is `unsafe` — the call
/// site attests every pre-compiled artifact is trusted.
#[derive(Debug)]
pub struct Precompiled;

/// Builds a [`Deployment`] from an optional programmatic [`Manifest`].
///
/// When no manifest is set, [`build`](Self::build) loads the path in
/// `OMNIA_CONFIG`.
///
/// The `P` typestate selects the artifact policy: the default
/// ([`WasmOnly`]) exposes a safe `build` that rejects pre-compiled (native)
/// artifacts; [`precompiled`](Self::precompiled) transitions to
/// [`Precompiled`], whose same-named `build` is `unsafe` because a
/// pre-compiled artifact is native code the caller must trust.
///
/// ```ignore
/// let deployment = DeploymentBuilder::new()
///     .manifest(Manifest::from_wasm(wasm))
///     .args(args)
///     .mode(mode)
///     .build::<StoreCtx>()
///     .await?;
/// ```
pub struct DeploymentBuilder<P = WasmOnly> {
    manifest: Option<Manifest>,
    args: Vec<String>,
    mode: Mode,
    allow_empty: bool,
    resolver: Option<Arc<dyn GuestResolver>>,
    http_fallback: Option<HttpFallback>,
    policy: PhantomData<fn() -> P>,
}

// Manual: the resolver and fallback are non-Debug trait objects.
impl<P> std::fmt::Debug for DeploymentBuilder<P> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("DeploymentBuilder")
            .field("manifest", &self.manifest)
            .field("args", &self.args)
            .field("mode", &self.mode)
            .field("allow_empty", &self.allow_empty)
            .field("resolver", &self.resolver.is_some())
            .field("http_fallback", &self.http_fallback.is_some())
            .finish_non_exhaustive()
    }
}

impl Default for DeploymentBuilder<WasmOnly> {
    fn default() -> Self {
        Self {
            manifest: None,
            args: Vec::new(),
            mode: Mode::default(),
            allow_empty: false,
            resolver: None,
            http_fallback: None,
            policy: PhantomData,
        }
    }
}

impl<P> DeploymentBuilder<P> {
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

    /// Mark the deployment as dynamically populated: the guest set may start
    /// empty and grow at run time via [`Runtime::register`](crate::Runtime::register)
    /// or resolve-on-miss (see [`resolver`](Self::resolver)).
    ///
    /// This only relaxes the "at least one guest" check — static trigger
    /// routing (HTTP/messaging/websocket/CLI) is built at boot; registered
    /// guests are reachable via host-mediated link dispatch, host→guest
    /// [`Dispatcher::invoke`](crate::Dispatcher::invoke), and — when an
    /// [`http_fallback`](Self::http_fallback) is installed — HTTP requests no
    /// static route matches.
    #[must_use]
    pub const fn dynamic(mut self) -> Self {
        self.allow_empty = true;
        self
    }

    /// Install a [`GuestResolver`] consulted on dispatch-path registry misses
    /// (resolve-on-miss).
    #[must_use]
    pub fn resolver(mut self, resolver: Arc<dyn GuestResolver>) -> Self {
        self.resolver = Some(resolver);
        self
    }

    /// Install the HTTP trigger fallback: maps a request path no static route
    /// matches to a guest identity, which then goes through the ordinary
    /// lookup (and hence resolve-on-miss).
    #[must_use]
    pub fn http_fallback<F>(mut self, fallback: F) -> Self
    where
        F: Fn(&str) -> Option<GuestId> + Send + Sync + 'static,
    {
        self.http_fallback = Some(Arc::new(fallback));
        self
    }

    /// Resolve the manifest and build the deployment under `policy`.
    async fn build_inner<T: WasiView + 'static>(
        self, policy: ArtifactPolicy,
    ) -> Result<Deployment<T>> {
        let manifest = if let Some(manifest) = self.manifest {
            manifest
        } else if self.allow_empty {
            // A dynamic deployment may start empty and register guests later.
            Manifest::new()
        } else {
            let config = env::var_os("OMNIA_CONFIG")
                .context("no deployment manifest supplied and OMNIA_CONFIG is unset")?;
            Manifest::from_config(config)?
        };
        manifest.validate(self.allow_empty)?;

        let plan = Plan {
            name: manifest.name().to_owned(),
            sources: manifest.sources()?,
            routes: manifest.routes(),
            links: manifest.link_interfaces(),
            preopens: manifest.preopens(),
            args: self.args,
            mode: self.mode,
            allow_empty: self.allow_empty,
            policy,
        };

        init_env(&plan.name)?;
        tracing::info!("initializing runtime");

        let mut deployment = Deployment::from_plan(plan).await?;
        deployment.resolver = self.resolver;
        deployment.http_fallback = self.http_fallback;
        Ok(deployment)
    }
}

impl DeploymentBuilder<WasmOnly> {
    /// Start a new builder with no source selected.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Admit pre-compiled (native) artifacts, making `build` `unsafe`.
    ///
    /// Only changes typestate; the trust attestation happens at the
    /// [`Precompiled`] `build` call site.
    #[must_use]
    pub fn precompiled(self) -> DeploymentBuilder<Precompiled> {
        DeploymentBuilder {
            manifest: self.manifest,
            args: self.args,
            mode: self.mode,
            allow_empty: self.allow_empty,
            resolver: self.resolver,
            http_fallback: self.http_fallback,
            policy: PhantomData,
        }
    }

    /// Resolve the manifest into a [`Deployment`].
    ///
    /// If no manifest was supplied, the path in `OMNIA_CONFIG` is loaded.
    /// Every guest must be raw component wasm; a pre-compiled (native)
    /// artifact is rejected — see [`precompiled`](Self::precompiled).
    ///
    /// # Errors
    ///
    /// Returns an error if no manifest resolves, the manifest is invalid, a
    /// guest names a pre-compiled artifact, or the deployment cannot be built.
    pub async fn build<T: WasiView + 'static>(self) -> Result<Deployment<T>> {
        self.build_inner(ArtifactPolicy::Reject).await
    }
}

impl DeploymentBuilder<Precompiled> {
    /// Resolve the manifest into a [`Deployment`], admitting pre-compiled
    /// artifacts.
    ///
    /// If no manifest was supplied, the path in `OMNIA_CONFIG` is loaded.
    ///
    /// # Safety
    ///
    /// Every pre-compiled path this builder's manifest names must identify
    /// trusted, immutable wasmtime output (`omnia compile` /
    /// [`wasmtime::component::Component::serialize`]). A pre-compiled
    /// artifact is native code: wasmtime's compatibility check is not an
    /// authenticity check, and tampered bytes can execute arbitrary code
    /// with host privileges.
    ///
    /// # Errors
    ///
    /// Returns an error if no manifest resolves, the manifest is invalid, or
    /// the deployment cannot be built.
    pub async unsafe fn build<T: WasiView + 'static>(self) -> Result<Deployment<T>> {
        self.build_inner(ArtifactPolicy::Trust).await
    }
}

/// A compiled set of WebAssembly components with their shared Linker, ready to
/// be [`host`]ed against WASI interfaces and assembled into a [`Registry`].
///
/// [`host`]: Self::host
pub struct Deployment<T: WasiView + 'static> {
    engine: Engine,
    linker: Linker<T>,
    options: RuntimeOptions,
    guests: Vec<LoadedGuest>,
    routes: Routes,
    // Guest links — the host-mediated interfaces.
    links: BTreeSet<Box<str>>,
    // Host-mediated dispatch selector.
    selector: Arc<dyn GuestSelector>,
    // Mount registry from resolved preopens in [`from_plan`](Self::from_plan).
    mounts: Arc<MountRegistry>,
    // Guest argv threaded into every store. Empty for long-lived servers; in
    // command mode the deployment name is prepended as `argv[0]`.
    args: Arc<Vec<String>>,
    // Whether this deployment runs a one-shot `wasi:cli` command.
    mode: Mode,
    // Whether the guest set may start empty and grow at run time.
    allow_empty: bool,
    // Resolve-on-miss hooks carried from the builder into `Runtime::new`.
    resolver: Option<Arc<dyn GuestResolver>>,
    http_fallback: Option<HttpFallback>,
}

impl<T: WasiView + 'static> Deployment<T> {
    /// Acquire every guest named in `plan` through its [`Source`] and pair them
    /// with the shared engine and WASI-linked linker.
    ///
    /// Acquisition runs through the async [`Source::load`] seam so a future
    /// source kind (an OCI pull) slots in without a parallel loading path.
    async fn from_plan(plan: Plan) -> Result<Self> {
        let (engine, linker, options) = engine_and_linker()?;

        // Open + identity-stamp every preopen once, here, so a misconfigured
        // mount fails fast at startup rather than per store.
        let mounts = Arc::new(MountRegistry::open(plan.preopens)?);

        // Guests load (and compile) in parallel; order still follows the plan.
        let loaded = futures::future::try_join_all(
            plan.sources.iter().map(|source| source.load(&engine, plan.policy)),
        )
        .await?;
        let guests = loaded.into_iter().flatten().collect();

        let args = if plan.mode.is_command() {
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
            mounts,
            args: Arc::new(args),
            mode: plan.mode,
            allow_empty: plan.allow_empty,
            resolver: None,
            http_fallback: None,
        })
    }
}

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
    pub fn selector(&mut self, selector: impl GuestSelector) -> &mut Self {
        self.selector = Arc::new(selector);
        self
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

    /// The builder-carried resolve-on-miss hooks, for `Runtime::new` to
    /// install before the deployment is consumed into a registry.
    pub(crate) fn resolve_hooks(&self) -> (Option<Arc<dyn GuestResolver>>, Option<HttpFallback>) {
        (self.resolver.clone(), self.http_fallback.clone())
    }

    /// Assemble the guest [`Registry`].
    ///
    /// Consumes the deployment: pre-instantiation happens once, here, after all
    /// hosts are linked — so no host can be linked after the guests are frozen.
    /// Per call only a fresh instantiate on a new store remains.
    ///
    /// # Errors
    ///
    /// Returns an error if host-mediated imports cannot be polyfilled, a
    /// component cannot be pre-instantiated, or the registry cannot be assembled.
    pub fn into_registry(self) -> Result<Registry<T>>
    where
        T: WrpcView,
    {
        let dispatch = DispatchHandle::new(
            self.selector,
            self.links,
            self.options.max_dispatch_depth,
            self.options.guest_timeout,
        );

        Registry::assemble(
            self.engine,
            self.linker,
            self.options,
            self.guests,
            self.routes,
            dispatch,
            self.allow_empty,
        )
    }
}

// Resolved deployment inputs shared by the manifest and single-file paths.
struct Plan {
    name: String,
    sources: Vec<Source>,
    routes: Routes,
    links: BTreeSet<Box<str>>,
    preopens: Vec<ResolvedPreopen>,
    args: Vec<String>,
    mode: Mode,
    allow_empty: bool,
    policy: ArtifactPolicy,
}

// Build the shared engine, WASI-linked linker, and runtime options.
fn engine_and_linker<T: WasiView + 'static>() -> Result<(Engine, Linker<T>, RuntimeOptions)> {
    let options = RuntimeOptions::load()?;
    let engine = Engine::new(&Config::from(&options))?;

    // register services with runtime's Linker
    let mut linker = Linker::new(&engine);
    wasmtime_wasi::p2::add_to_linker_async(&mut linker)?;
    wasmtime_wasi::p3::add_to_linker(&mut linker)?;

    Ok((engine, linker, options))
}

// Initialize telemetry and the `COMPONENT` environment variable for the runtime.
//
// Telemetry (a process-global tracing subscriber) initializes once; later
// deployments in the same process — embedders or a multi-deployment test
// suite — reuse the first initialization.
fn init_env(name: &str) -> Result<()> {
    static TELEMETRY: std::sync::OnceLock<()> = std::sync::OnceLock::new();

    if env::var_os("COMPONENT").is_none() {
        // SAFETY: Environment variable modification is safe here because:
        // 1. This runs during single-threaded initialization
        // 2. Backend clients that depend on these vars are created after this
        unsafe {
            env::set_var("COMPONENT", name);
        };
    }

    if TELEMETRY.set(()).is_err() {
        return Ok(());
    }

    let mut builder = Telemetry::new(name);
    if let Ok(endpoint) = env::var("OTEL_GRPC_URL") {
        builder = builder.endpoint(endpoint);
    } else {
        tracing::debug!("OTEL_GRPC_URL unset; using OpenTelemetry defaults");
    }
    builder.build().context("initializing telemetry")
}

#[cfg(test)]
mod tests {
    use wasmtime::{Config, Engine};

    use crate::RuntimeOptions;

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
            pool_decommit_batch_size: Some(8),
            ..RuntimeOptions::load().expect("should load")
        };
        Engine::new(&Config::from(&options))
            .expect("decoupled multi-memory pooling config should build an engine");
    }

    #[test]
    fn builds_no_pooling() {
        let options = RuntimeOptions {
            pooling: false,
            ..RuntimeOptions::load().expect("should load")
        };
        Engine::new(&Config::from(&options)).expect("non-pooling config should build an engine");
    }
}
