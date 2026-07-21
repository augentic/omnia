//! Deployment lifecycle: [`Backends`], [`Wiring`], [`Runtime`], [`run`], and [`ExitStatus`].

mod command;

use std::collections::HashMap;
use std::env;
use std::future::Future;
use std::process::ExitCode;
use std::sync::{Arc, Mutex, OnceLock, PoisonError, Weak};
use std::time::Duration;

use anyhow::{Context as _, Result};
use clap::Parser as _;
use futures::FutureExt as _;
use futures::future::{BoxFuture, Shared};
use wasmtime::component::types::ComponentItem;
use wasmtime::component::{Component, Instance, InstancePre};
use wasmtime::{Engine, Store};

use crate::cli::{Cli, Command};
use crate::deployment::GuestArtifact;
use crate::dispatch::{
    EnsureError, GuestResolver, HttpFallback, ResolveHook, serve_guest, serve_links,
};
use crate::host::FutureResult;
use crate::mount::MountRegistry;
use crate::registry::{Guest, GuestId};
use crate::store::HasLimits;
use crate::{
    Deployment, DeploymentBuilder, Dispatcher, Manifest, Registry, RuntimeOptions, StoreBase,
    StoreCtx,
};

/// A deployment's connected backend bundle, threaded into [`Runtime`].
///
/// The `runtime!` macro generates the concrete bundle (one field per declared
/// backend) and this impl, whose [`connect`](Self::connect) connects every
/// backend concurrently — the work the macro previously inlined as a
/// `tokio::try_join!` in the generated `Runtime::new`. A deployment that wires
/// no backends uses the [`()`](unit) bundle below, so [`Runtime`] needs no
/// special empty case.
pub trait Backends: Clone + Send + Sync + 'static {
    /// Connect every backend in the bundle.
    ///
    /// # Errors
    ///
    /// Returns the first backend connection error.
    fn connect() -> impl Future<Output = Result<Self>>;
}

/// The zero-backend bundle: a deployment that links only backend-less hosts
/// (such as a `mode: command` `wasi:cli` deployment) connects nothing.
impl Backends for () {
    async fn connect() -> Result<Self> {
        Ok(())
    }
}

/// How a deployment is driven after bootstrap.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Mode {
    /// Await trigger servers until shutdown.
    #[default]
    Server,
    /// Drive `wasi:cli/run` once; trigger servers run in the background.
    Command,
}

impl Mode {
    /// Whether guest argv is shaped for a one-shot `wasi:cli` command.
    #[must_use]
    pub const fn is_command(self) -> bool {
        matches!(self, Self::Command)
    }
}

/// Host linking and trigger-server startup for a deployment.
pub trait Wiring<B: Backends> {
    /// Link every declared host into the deployment linker.
    ///
    /// # Errors
    ///
    /// Returns an error if a host cannot be added to the linker.
    fn link(deployment: &mut Deployment<StoreCtx<B>>) -> Result<()>;

    /// Run every declared long-lived trigger server concurrently.
    fn serve(runtime: &Runtime<B>) -> impl std::future::Future<Output = Result<()>> + Send;
}

/// A runtime's compiled-in deployment fallback, used only when the CLI
/// supplies no source.
///
/// The `runtime!` macro emits [`Path`](Self::Path) for its `config:` field and
/// [`Inline`](Self::Inline) for its inline manifest keys (`guests`, `mounts`,
/// `link`, `routes`).
#[derive(Clone, Debug)]
pub enum DefaultManifest {
    /// A default manifest path, loaded only when the fallback is reached.
    Path(std::path::PathBuf),
    /// A manifest value assembled at compile time.
    Inline(Manifest),
}

impl DefaultManifest {
    /// Resolve the fallback into a manifest, loading the file for the path kind.
    fn into_manifest(self) -> Result<Manifest> {
        match self {
            Self::Path(path) => Manifest::from_config(path),
            Self::Inline(manifest) => Ok(manifest),
        }
    }
}

/// CLI entry point for generated `main` functions.
///
/// `default_manifest` is the runtime's compiled-in deployment fallback (the
/// `runtime!` macro's `config:` field or inline manifest keys), used only when
/// the CLI supplies no source.
#[doc(hidden)]
pub async fn main<B, H>(mode: Mode, default_manifest: Option<DefaultManifest>) -> ExitCode
where
    B: Backends,
    H: Wiring<B>,
{
    match Cli::parse().command {
        Command::Run {
            wasm,
            config,
            mounts,
            links,
            args,
        } => {
            let config = config.or_else(|| env::var_os("OMNIA_CONFIG").map(Into::into));
            let manifest = config
                .map_or_else(
                    || {
                        wasm.map_or_else(
                            || {
                                default_manifest
                                    .context(
                                        "no guest specified: pass a <wasm> path, or --config \
                                         <omnia.toml> (or set OMNIA_CONFIG)",
                                    )
                                    .and_then(DefaultManifest::into_manifest)
                            },
                            |wasm| Ok(Manifest::from_wasm(wasm)),
                        )
                    },
                    Manifest::from_config,
                )
                .map(|manifest| manifest.mounts(mounts).links(links));

            let result = match manifest {
                Ok(manifest) => {
                    // The CLI path admits pre-compiled artifacts: manifests and
                    // `.bin` paths given to the binary are trusted operator
                    // inputs (docs/security-model.md).
                    let builder = DeploymentBuilder::new()
                        .manifest(manifest)
                        .args(args)
                        .mode(mode)
                        .precompiled();
                    // SAFETY: the operator running this binary chose the
                    // manifest and artifact paths; pre-compiled artifacts are
                    // documented trusted inputs produced by `omnia compile`.
                    unsafe { run_precompiled::<B, H>(builder) }.await
                }
                Err(error) => Err(error),
            };
            match result {
                Ok(status) => status.into(),
                Err(error) => {
                    eprintln!("{error:#}");
                    ExitCode::FAILURE
                }
            }
        }
        #[cfg(feature = "jit")]
        Command::Compile { .. } => {
            eprintln!(
                "the generated `main` only supports `run`; supply a custom `main` for other \
                 subcommands"
            );
            ExitCode::FAILURE
        }
    }
}

/// Build runtime state, bootstrap it, then run command mode or every trigger server.
///
/// The default ([`WasmOnly`](crate::WasmOnly)) builder only loads raw wasm; a
/// deployment of trusted pre-compiled artifacts builds its [`Deployment`]
/// through the [`Precompiled`](crate::Precompiled) typestate's unsafe `build`
/// (as the generated CLI `main` does).
///
/// # Errors
///
/// Returns an error if the deployment cannot be built, runtime state cannot be
/// assembled, bootstrap fails, or a trigger server exits with an error.
pub async fn run<B, H>(builder: DeploymentBuilder) -> Result<ExitStatus>
where
    B: Backends,
    H: Wiring<B>,
{
    let deployment = builder.build::<StoreCtx<B>>().await.context("building runtime")?;
    drive::<B, H>(deployment).await
}

/// [`run`] for a deployment of trusted pre-compiled artifacts.
///
/// The [`Precompiled`](crate::Precompiled) parameter means a raw/default
/// builder cannot select this path by accident — the caller must transition
/// through [`DeploymentBuilder::precompiled`](crate::DeploymentBuilder::precompiled)
/// first.
///
/// # Safety
///
/// Every pre-compiled path the builder's manifest names must identify
/// trusted, immutable wasmtime output (`omnia compile`); see
/// [`DeploymentBuilder::build`](crate::DeploymentBuilder) in the
/// `Precompiled` typestate.
///
/// # Errors
///
/// Returns an error if the deployment cannot be built, runtime state cannot be
/// assembled, bootstrap fails, or a trigger server exits with an error.
pub async unsafe fn run_precompiled<B, H>(
    builder: DeploymentBuilder<crate::Precompiled>,
) -> Result<ExitStatus>
where
    B: Backends,
    H: Wiring<B>,
{
    // SAFETY: forwarded — this function's own contract is exactly the
    // typestate build's contract.
    let deployment = unsafe { builder.build::<StoreCtx<B>>() }.await.context("building runtime")?;
    drive::<B, H>(deployment).await
}

/// Drive an already-built deployment: assemble the runtime, start background
/// tasks, then run command mode or every trigger server.
async fn drive<B, H>(deployment: Deployment<StoreCtx<B>>) -> Result<ExitStatus>
where
    B: Backends,
    H: Wiring<B>,
{
    let mode = deployment.mode();

    let runtime = Runtime::<B>::new(deployment, H::link).await.context("assembling runtime")?;

    // start background tasks
    drive_epoch(runtime.registry().engine().clone(), runtime.options().epoch_tick);
    sample_pool(runtime.registry().engine().clone(), runtime.options().pool_metrics_interval);

    log_bootstrap_complete(&runtime, mode);

    match mode {
        Mode::Command => {
            let servers_runtime = runtime.clone();
            tokio::spawn(async move {
                if let Err(error) = H::serve(&servers_runtime).await {
                    tracing::error!(%error, "trigger server exited with error");
                }
            });
            command::drive(&runtime).await
        }
        Mode::Server => {
            H::serve(&runtime).await?;
            Ok(ExitStatus::SUCCESS)
        }
    }
}

fn log_bootstrap_complete<B>(runtime: &Runtime<B>, mode: Mode)
where
    B: Clone + Send + Sync + 'static,
{
    tracing::info!(
        mode = if mode.is_command() { "command" } else { "server" },
        guests = runtime.registry().len(),
        component = env::var("COMPONENT").unwrap_or_else(|_| "unknown".into()),
        "omnia ready",
    );
}

fn drive_epoch(engine: Engine, tick: Duration) {
    tokio::spawn(async move {
        let mut interval = tokio::time::interval(tick);
        loop {
            interval.tick().await;
            engine.increment_epoch();
        }
    });
}

fn sample_pool(engine: Engine, interval: Duration) {
    if interval.is_zero() {
        return;
    }

    tokio::spawn(async move {
        let mut ticker = tokio::time::interval(interval);
        loop {
            ticker.tick().await;

            let Some(metrics) = engine.pooling_allocator_metrics() else {
                break;
            };

            tracing::debug!(
                gauge.pool_core_instances = metrics.core_instances(),
                gauge.pool_component_instances = metrics.component_instances(),
                gauge.pool_memories = metrics.memories() as u64,
                gauge.pool_tables = metrics.tables() as u64,
                gauge.pool_stacks = metrics.stacks() as u64,
                gauge.pool_unused_warm_memories = u64::from(metrics.unused_warm_memories()),
                gauge.pool_unused_memory_bytes_resident =
                    metrics.unused_memory_bytes_resident() as u64,
            );
        }
    });
}

/// Guest exit code. [`code_u8`](Self::code_u8) and [`ExitCode`](std::process::ExitCode)
/// keep only the low byte (POSIX semantics).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ExitStatus(i32);

impl ExitStatus {
    /// Exit code `0`.
    pub const SUCCESS: Self = Self(0);

    /// Full `i32` exit code from the guest.
    #[must_use]
    pub const fn code(self) -> i32 {
        self.0
    }

    /// Low byte of the exit code (POSIX process status).
    #[must_use]
    pub const fn code_u8(self) -> u8 {
        self.0.to_le_bytes()[0]
    }
}

impl From<i32> for ExitStatus {
    fn from(code: i32) -> Self {
        Self(code)
    }
}

impl From<ExitStatus> for std::process::ExitCode {
    fn from(status: ExitStatus) -> Self {
        Self::from(status.code_u8())
    }
}

/// Connected host runtime: registry, argv, mounts, and backend bundle.
///
/// A thin handle over shared state: `clone()` bumps two reference counts, so
/// the per-request and per-message handler clones never copy the backend
/// bundle.
pub struct Runtime<B: 'static> {
    inner: Arc<RuntimeInner<B>>,
    // Cached host→guest dispatch capability, built once per runtime so
    // `store()` hands out clones instead of allocating one per store.
    dispatcher: Arc<dyn Dispatcher>,
}

/// One resolve-on-miss flight: a shared future every concurrent waiter for
/// the same missing identity awaits, so the resolver runs once per miss and
/// all waiters share the outcome — negatives included.
type Flight<B> = Shared<BoxFuture<'static, Result<Arc<Guest<StoreCtx<B>>>, EnsureError>>>;

struct RuntimeInner<B: 'static> {
    registry: Arc<Registry<StoreCtx<B>>>,
    args: Arc<Vec<String>>,
    mounts: Arc<MountRegistry>,
    backends: B,
    // Resolve-on-miss seam (RFC guest-resolution §4.5). Install-once: hooks
    // ride the deployment builder (or the `from_parts` chainable setters) and
    // never change for the life of the runtime.
    resolver: OnceLock<Arc<dyn GuestResolver>>,
    http_fallback: OnceLock<HttpFallback>,
    // In-flight resolutions by identity. An entry lives exactly as long as
    // its flight: inserted when the flight starts, removed when its outcome
    // is computed — nothing is cached across flights.
    flights: Mutex<HashMap<GuestId, Flight<B>>>,
}

impl<B: 'static> RuntimeInner<B> {
    fn new(
        registry: Arc<Registry<StoreCtx<B>>>, args: Arc<Vec<String>>, mounts: Arc<MountRegistry>,
        backends: B,
    ) -> Self {
        Self {
            registry,
            args,
            mounts,
            backends,
            resolver: OnceLock::new(),
            http_fallback: OnceLock::new(),
            flights: Mutex::new(HashMap::new()),
        }
    }
}

/// [`Dispatcher`] over the runtime's shared state.
///
/// A separate type (rather than `Runtime` itself) so the cached
/// `Arc<dyn Dispatcher>` inside [`Runtime`] does not create a reference cycle.
pub struct RuntimeDispatcher<B: 'static> {
    inner: Arc<RuntimeInner<B>>,
}

impl<B: Clone + Send + Sync + 'static> RuntimeDispatcher<B> {
    /// Rehydrate a full runtime handle for a dispatched call.
    pub fn runtime(&self) -> Runtime<B> {
        Runtime::with_inner(Arc::clone(&self.inner))
    }
}

impl<B: Backends> Runtime<B> {
    /// Link hosts, connect backends, assemble the guest registry, and wire
    /// the host-mediated link serve side.
    ///
    /// # Errors
    ///
    /// Returns an error if host linking, backend connection, registry
    /// assembly, or link serve wiring fails.
    pub async fn new<L>(mut deployment: Deployment<StoreCtx<B>>, link: L) -> Result<Self>
    where
        L: FnOnce(&mut Deployment<StoreCtx<B>>) -> Result<()>,
    {
        let args = Arc::new(deployment.args().to_vec());
        link(&mut deployment).context("linking hosts")?;
        let backends = B::connect().await.context("connecting backends")?;
        let mounts = deployment.mounts();
        let (resolver, http_fallback) = deployment.resolve_hooks();

        let runtime = Self::with_inner(Arc::new(RuntimeInner::new(
            Arc::new(deployment.into_registry().context("assembling registry")?),
            args,
            mounts,
            backends,
        )));
        if let Some(resolver) = resolver {
            runtime.install_resolver(resolver);
        }
        if let Some(fallback) = http_fallback {
            runtime.install_http_fallback(fallback);
        }
        serve_links(&runtime).await.context("wiring host-mediated link serve side")?;
        Ok(runtime)
    }
}

// Manual: `StoreCtx<B>` is not `Clone`; both fields are `Arc`-backed.
impl<B: Clone + Send + Sync + 'static> Clone for Runtime<B> {
    fn clone(&self) -> Self {
        Self {
            inner: Arc::clone(&self.inner),
            dispatcher: Arc::clone(&self.dispatcher),
        }
    }
}

impl<B: Clone + Send + Sync + 'static> Runtime<B> {
    fn with_inner(inner: Arc<RuntimeInner<B>>) -> Self {
        let dispatcher = Arc::new(RuntimeDispatcher {
            inner: Arc::clone(&inner),
        });
        Self { inner, dispatcher }
    }

    /// Build a runtime from an already-assembled registry and backend bundle.
    ///
    /// Low-level constructor: unlike [`Runtime::new`] it does not wire the
    /// host-mediated link serve side — a caller whose deployment declares
    /// `link` interfaces must run [`serve_links`] itself before dispatching.
    #[must_use]
    pub fn from_parts(
        registry: Arc<Registry<StoreCtx<B>>>, args: Vec<String>, mounts: Arc<MountRegistry>,
        backends: B,
    ) -> Self {
        Self::with_inner(Arc::new(RuntimeInner::new(registry, Arc::new(args), mounts, backends)))
    }

    /// Install a [`GuestResolver`] consulted on dispatch-path registry misses
    /// (resolve-on-miss), chainable after [`from_parts`](Self::from_parts).
    ///
    /// Deployments built through [`DeploymentBuilder`] supply the resolver via
    /// [`DeploymentBuilder::resolver`] instead. Install-once: a second
    /// resolver is ignored with a warning.
    #[must_use]
    pub fn with_resolver(self, resolver: Arc<dyn GuestResolver>) -> Self {
        self.install_resolver(resolver);
        self
    }

    /// Install an [`HttpFallback`] mapping unrouted request paths to guest
    /// identities, chainable after [`from_parts`](Self::from_parts).
    ///
    /// Deployments built through [`DeploymentBuilder`] supply the fallback via
    /// [`DeploymentBuilder::http_fallback`] instead. Install-once: a second
    /// fallback is ignored with a warning.
    #[must_use]
    pub fn with_http_fallback<F>(self, fallback: F) -> Self
    where
        F: Fn(&str) -> Option<GuestId> + Send + Sync + 'static,
    {
        self.install_http_fallback(Arc::new(fallback));
        self
    }

    /// The installed HTTP trigger fallback, if any.
    #[must_use]
    pub fn http_fallback(&self) -> Option<HttpFallback> {
        self.inner.http_fallback.get().cloned()
    }

    fn install_resolver(&self, resolver: Arc<dyn GuestResolver>) {
        if self.inner.resolver.set(resolver).is_err() {
            tracing::warn!("guest resolver already installed; ignoring");
            return;
        }
        // The erased link-path hook holds a weak back-reference: the strong
        // chain RuntimeInner -> Registry -> DispatchHandle -> hook would
        // otherwise cycle.
        self.registry().dispatch().install_resolve_hook(Box::new(RuntimeResolveHook {
            inner: Arc::downgrade(&self.inner),
        }));
    }

    fn install_http_fallback(&self, fallback: HttpFallback) {
        if self.inner.http_fallback.set(fallback).is_err() {
            tracing::warn!("http fallback already installed; ignoring");
        }
    }

    /// Guest registry.
    #[must_use]
    pub fn registry(&self) -> &Registry<StoreCtx<B>> {
        &self.inner.registry
    }

    /// The cached host→guest dispatch capability — the same handle
    /// every store context carries, for host-side callers (tests,
    /// embedders) that invoke a guest export directly.
    #[must_use]
    pub fn dispatcher(&self) -> Arc<dyn Dispatcher> {
        Arc::clone(&self.dispatcher)
    }

    /// Runtime options from the environment.
    #[must_use]
    pub fn options(&self) -> &RuntimeOptions {
        self.registry().options()
    }

    /// Fresh per-guest store context.
    #[must_use]
    pub fn store(&self) -> StoreCtx<B> {
        let base = StoreBase::builder()
            .options(self.options())
            .dispatcher(Arc::clone(&self.dispatcher))
            .args(Arc::clone(&self.inner.args))
            .mounts(Arc::clone(&self.inner.mounts))
            .build();
        StoreCtx {
            base,
            backends: self.inner.backends.clone(),
        }
    }

    /// Store with epoch deadline, optional fuel, and memory limiter installed.
    #[must_use]
    pub fn build_store(&self, data: StoreCtx<B>) -> Store<StoreCtx<B>> {
        let options = self.options();
        let mut store = Store::new(self.registry().engine(), data);

        store.set_epoch_deadline(1);
        store.epoch_deadline_async_yield_and_update(1);

        if options.max_fuel > 0
            && let Err(error) = store.set_fuel(options.max_fuel)
        {
            tracing::warn!(%error, "failed to set fuel budget");
        }

        store.limiter(|ctx| ctx.limits());
        store
    }

    /// Instantiate a guest component into `store`.
    ///
    /// # Errors
    ///
    /// Returns an error if the component cannot be instantiated.
    pub async fn instantiate(
        &self, instance_pre: &InstancePre<StoreCtx<B>>, store: &mut Store<StoreCtx<B>>,
    ) -> Result<Instance> {
        let instance = instance_pre.instantiate_async(store).await?;
        tracing::debug!("component instantiated");
        Ok(instance)
    }

    /// Register a guest at run time: load `artifact`, pre-instantiate it
    /// against the shared host set, wire its host-mediated link serve side,
    /// then publish entry and endpoint as one atomic lifecycle transition —
    /// no dispatch can ever resolve the entry and miss the endpoint, or vice
    /// versa.
    ///
    /// The identity is opaque and must not already be registered; an upgrade
    /// is [`deregister`](Self::deregister) + `register` (or a new id). A
    /// failed registration leaves no partial state.
    ///
    /// # Errors
    ///
    /// Returns an error if `id` is already registered, the artifact cannot be
    /// loaded, the component's imports exceed the deployment's linked host set
    /// and `link` union, or its linked exports cannot be served.
    pub async fn register(&self, id: impl Into<GuestId>, artifact: GuestArtifact) -> Result<()> {
        self.register_inner(id.into(), artifact, None).await
    }

    /// [`register`](Self::register) internals, with an optional expected
    /// export the loaded component must satisfy — the resolve-on-miss path
    /// sets it because a resolver's answer is not trusted to be well-shaped
    /// (an unvalidated publish of a link target would create an entry whose
    /// endpoint the retry misses forever). Validation failure happens before
    /// serve/publish, so it leaves no partial state.
    async fn register_inner(
        &self, id: GuestId, artifact: GuestArtifact, expected_export: Option<&str>,
    ) -> Result<()> {
        let registry = self.registry();

        // Early occupancy check to skip the load/serve work; the publish below
        // re-checks transactionally, so a racing registration cannot slip in.
        anyhow::ensure!(registry.get(&id).is_none(), "guest `{id}` is already registered");

        let component = artifact
            .load(registry.engine())
            .await
            .with_context(|| format!("loading guest `{id}`"))?;
        if let Some(export) = expected_export {
            anyhow::ensure!(
                exports_instance(&component, registry.engine(), export),
                "guest `{id}` does not export interface `{export}`"
            );
        }
        let instance_pre = registry.instantiate_late(&id, &component)?;
        let guest = Guest::local(id.clone(), instance_pre);

        // Wire the guest's linked exports (if any); publish then makes the
        // endpoint and the registry entry observable in one atomic step. If
        // publish refuses (a racing registration won), dropping the unused
        // server winds its drain tasks down.
        let server = serve_guest(self, &guest)
            .await
            .with_context(|| format!("serving guest `{id}` link exports"))?;
        registry.publish(guest, server)?;

        tracing::info!(guest = %id, "guest registered");
        Ok(())
    }

    /// Return the registered guest for `id`, faulting it in through the
    /// installed [`GuestResolver`] on a miss (resolve-on-miss).
    ///
    /// A hit returns the entry directly. On a miss with a resolver installed,
    /// the call joins (or starts) the per-identity single flight: resolve →
    /// validate `expected_export` → register through the ordinary internals →
    /// return the entry. Every concurrent waiter shares the flight's outcome
    /// — negatives included — and no negative outcome is cached across
    /// flights.
    ///
    /// # Errors
    ///
    /// Returns [`EnsureError::Unresolved`] when nothing supplies the guest
    /// (no resolver, or the resolver answered `Ok(None)`),
    /// [`EnsureError::ResolveFailed`] when resolution or the subsequent
    /// registration failed, and [`EnsureError::ExportMismatch`] when a
    /// concurrently registered component lacks `expected_export`.
    pub async fn ensure_guest(
        &self, id: &GuestId, expected_export: &str,
    ) -> Result<Arc<Guest<StoreCtx<B>>>, EnsureError> {
        if let Some(guest) = self.registry().get(id) {
            return Ok(guest);
        }
        let Some(resolver) = self.inner.resolver.get() else {
            return Err(EnsureError::Unresolved(id.clone()));
        };
        self.join_or_start_flight(id, expected_export, Arc::clone(resolver)).await
    }

    /// Join the in-flight resolution for `id`, or start one. The flight
    /// future removes its own map entry once the outcome is computed, so a
    /// later miss starts a fresh flight (negatives are never cached).
    fn join_or_start_flight(
        &self, id: &GuestId, expected_export: &str, resolver: Arc<dyn GuestResolver>,
    ) -> Flight<B> {
        let mut flights = self.inner.flights.lock().unwrap_or_else(PoisonError::into_inner);
        if let Some(flight) = flights.get(id) {
            return flight.clone();
        }

        let runtime = self.clone();
        let flight_id = id.clone();
        let export = expected_export.to_owned();
        let flight: Flight<B> = async move {
            let outcome = run_flight(&runtime, resolver.as_ref(), &flight_id, &export).await;
            // The entry is still ours (a new flight for this id cannot start
            // while it is present), so removal here both ends the flight and
            // opens the door for the next miss.
            runtime.inner.flights.lock().unwrap_or_else(PoisonError::into_inner).remove(&flight_id);
            outcome
        }
        .boxed()
        .shared();
        flights.insert(id.clone(), flight.clone());
        flight
    }

    /// Remove a dynamically registered guest. New dispatches to `id` fail as
    /// unregistered; in-flight calls complete on the instance they hold
    /// (instance-per-call). Static deployment entries are refused.
    ///
    /// # Errors
    ///
    /// Returns an error if `id` names a static `[[guest]]` entry or is not
    /// registered.
    pub fn deregister(&self, id: &GuestId) -> Result<()> {
        self.registry().remove(id)?;
        tracing::info!(guest = %id, "guest deregistered");
        Ok(())
    }
}

/// One resolve-on-miss flight: consult the resolver, validate and register
/// its artifact, and return the registered entry.
async fn run_flight<B: Clone + Send + Sync + 'static>(
    runtime: &Runtime<B>, resolver: &dyn GuestResolver, id: &GuestId, expected_export: &str,
) -> Result<Arc<Guest<StoreCtx<B>>>, EnsureError> {
    let answer =
        resolver.resolve(id.clone(), expected_export.to_owned()).await.map_err(|error| {
            let error = error.context(format!("resolving guest `{id}`"));
            tracing::error!(guest = %id, "guest resolution failed: {error:#}");
            EnsureError::ResolveFailed(Arc::new(error))
        })?;
    let Some(artifact) = answer else {
        tracing::debug!(guest = %id, "resolver has no component for guest");
        return Err(EnsureError::Unresolved(id.clone()));
    };

    let raced = match runtime.register_inner(id.clone(), artifact, Some(expected_export)).await {
        Ok(()) => false,
        // Losing the publish race to a concurrent direct `register(id)` is
        // success — an entry exists; any other failure with no entry is real.
        Err(_) if runtime.registry().get(id).is_some() => true,
        Err(error) => {
            let error = error.context(format!("registering resolved guest `{id}`"));
            tracing::error!(guest = %id, "guest resolution failed: {error:#}");
            return Err(EnsureError::ResolveFailed(Arc::new(error)));
        }
    };

    let guest = runtime.registry().get(id).ok_or_else(|| {
        // Deregistered between publish and this lookup; the next miss starts
        // a fresh flight.
        EnsureError::Unresolved(id.clone())
    })?;
    // Our own registration was validated pre-publish; a race winner's
    // component is unvetted, so check it satisfies the dispatch site.
    if raced && !exports_instance(guest.component(), runtime.registry().engine(), expected_export) {
        return Err(EnsureError::ExportMismatch {
            guest: id.clone(),
            export: expected_export.to_owned(),
        });
    }
    Ok(guest)
}

/// Whether `component` exports an instance (interface) named `export`,
/// tolerating a versioned export name (`wasi:http/incoming-handler@0.3.0`
/// satisfies `wasi:http/incoming-handler`).
fn exports_instance(component: &Component, engine: &Engine, export: &str) -> bool {
    component.component_type().exports(engine).any(|(name, item)| {
        matches!(item.ty, ComponentItem::ComponentInstance(_))
            && (name == export
                || name.strip_prefix(export).is_some_and(|rest| rest.starts_with('@')))
    })
}

/// The erased link-path resolve hook: rehydrates a [`Runtime`] from a weak
/// back-reference and delegates to [`Runtime::ensure_guest`].
struct RuntimeResolveHook<B: 'static> {
    inner: Weak<RuntimeInner<B>>,
}

impl<B: Clone + Send + Sync + 'static> ResolveHook for RuntimeResolveHook<B> {
    fn ensure(&self, guest: &GuestId, expected_export: &str) -> FutureResult<()> {
        let inner = Weak::clone(&self.inner);
        let guest = guest.clone();
        let expected_export = expected_export.to_owned();
        async move {
            let inner = inner.upgrade().context("runtime dropped during resolve")?;
            Runtime::with_inner(inner)
                .ensure_guest(&guest, &expected_export)
                .await
                .map(|_| ())
                .map_err(anyhow::Error::from)
        }
        .boxed()
    }
}

#[cfg(test)]
mod tests {
    use super::ExitStatus;

    #[test]
    fn code_u8_keeps_low_byte() {
        // The POSIX low-byte truncation is the only non-trivial ExitStatus logic.
        assert_eq!(ExitStatus::from(2).code(), 2);
        assert_eq!(ExitStatus::from(256).code_u8(), 0);
        assert_eq!(ExitStatus::from(257).code_u8(), 1);
        assert_eq!(ExitStatus::from(-1).code_u8(), 255);
    }
}
