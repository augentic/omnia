//! Deployment lifecycle: [`Backends`], [`Wiring`], [`Runtime`], [`run`], and [`ExitStatus`].

mod command;
mod entry;

use std::collections::HashMap;
use std::env;
use std::future::Future;
use std::process::ExitCode;
use std::sync::{Arc, Mutex, OnceLock, PoisonError, Weak};
use std::time::Duration;

use anyhow::{Context as _, Result};
pub use entry::{MainOptions, ManifestSource};
use futures::FutureExt as _;
use futures::future::{BoxFuture, Shared};
use wasmtime::component::types::ComponentItem;
use wasmtime::component::{Component, Instance, InstancePre};
use wasmtime::{Engine, Store};

use crate::deployment::GuestArtifact;
use crate::dispatch::{
    EnsureError, GuestResolver, HttpPaths, ResolveHook, serve_guest, serve_links,
};
use crate::host::FutureResult;
use crate::mount::MountRegistry;
use crate::registry::{Guest, GuestId, HttpRoutes, RoutingPolicy, TriggerRouter};
use crate::store::{HasLimits, merged_env};
use crate::{
    Deployment, DeploymentBuilder, Dispatcher, Registry, RuntimeOptions, StoreBase, StoreCtx,
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

/// Entry point for generated `main` functions.
///
/// `options` carries the deployment the `runtime!` macro compiled in: mode,
/// manifest source, resolver, invocation shape, and command guest. Without
/// the macro's `program:` key this parses the standard
/// `run [wasm] [--config] -- args…` grammar; with it, argv passes to the
/// guest verbatim except the reserved host log flags (`--debug` / `--quiet`),
/// which select the telemetry [`LogMode`](crate::LogMode).
#[doc(hidden)]
pub async fn main<B, H>(options: MainOptions) -> ExitCode
where
    B: Backends,
    H: Wiring<B>,
{
    let plan = match entry::plan(options, env::args_os(), env::var_os("OMNIA_CONFIG")) {
        Ok(plan) => plan,
        #[cfg(feature = "cli")]
        Err(entry::PlanError::Usage(error)) => error.exit(),
        Err(entry::PlanError::Fatal(error)) => {
            eprintln!("{error:#}");
            return ExitCode::FAILURE;
        }
    };
    // The generated entry point admits pre-compiled artifacts: manifests and
    // `.bin` paths given to (or compiled into) the binary are trusted
    // operator inputs (docs/security-model.md).
    let builder = plan.into_builder().precompiled();
    // SAFETY: the operator running this binary chose the manifest and
    // artifact paths; pre-compiled artifacts are documented trusted inputs
    // produced by `omnia compile`.
    match unsafe { run_precompiled::<B, H>(builder) }.await {
        Ok(status) => status.into(),
        Err(error) => {
            eprintln!("{error:#}");
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

    // Background tasks hold Engine clones; abort them when the drive
    // completes so a finished deployment releases its engine (and the pooling
    // allocator's large virtual reservation) instead of leaking it into the
    // host process.
    let epoch = drive_epoch(runtime.registry().engine().clone(), runtime.options().epoch_tick);
    let pool =
        sample_pool(runtime.registry().engine().clone(), runtime.options().pool_metrics_interval);

    log_ready(&runtime, mode);

    let outcome = match mode {
        Mode::Command => {
            let servers_runtime = runtime.clone();
            tokio::spawn(async move {
                if let Err(error) = H::serve(&servers_runtime).await {
                    tracing::error!(%error, "trigger server exited with error");
                }
            });
            command::drive(&runtime).await
        }
        Mode::Server => H::serve(&runtime).await.map(|()| ExitStatus::SUCCESS),
    };

    epoch.abort();
    if let Some(pool) = pool {
        pool.abort();
    }
    // Drop every link-serve endpoint: the drain tasks hold Runtime clones, so
    // leaving them running would pin the engine past the deployment's life.
    runtime.shutdown();
    // Push batch-queued spans and metrics to the exporters so they survive
    // fast command-mode exits.
    crate::telemetry::flush();
    outcome
}

fn log_ready<B>(runtime: &Runtime<B>, mode: Mode)
where
    B: Clone + Send + Sync + 'static,
{
    tracing::info!(
        mode = if mode.is_command() { "command" } else { "server" },
        guests = runtime.registry().len(),
        component = runtime.name(),
        "omnia ready",
    );
}

fn drive_epoch(engine: Engine, tick: Duration) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        let mut interval = tokio::time::interval(tick);
        loop {
            interval.tick().await;
            engine.increment_epoch();
        }
    })
}

fn sample_pool(engine: Engine, interval: Duration) -> Option<tokio::task::JoinHandle<()>> {
    if interval.is_zero() {
        return None;
    }

    let handle = tokio::spawn(async move {
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
    Some(handle)
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

/// Why [`Runtime::route_http`] produced no guest for a request path.
///
/// Typed so the HTTP trigger server and the testkit harness map late-routing
/// outcomes to the same wire semantics: an ordinary miss is a 404, a claimed
/// route that cannot be served is a 500.
#[derive(Debug)]
pub enum RouteRefusal {
    /// An ordinary unmatched request (a 404 on the wire): no [`HttpPaths`]
    /// hook is installed, the hook declined the path, or the identity it
    /// named is one nothing supplies (the resolver's definitive miss).
    NotFound,
    /// The hook claimed the path but serving it faulted — resolution failed,
    /// or the guest lacks the handler export (a 500 on the wire, never an
    /// ordinary miss).
    Failed(EnsureError),
}

impl std::fmt::Display for RouteRefusal {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NotFound => write!(f, "no route matched path"),
            Self::Failed(_) => write!(f, "claimed route cannot be served"),
        }
    }
}

impl std::error::Error for RouteRefusal {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::NotFound => None,
            Self::Failed(source) => Some(source),
        }
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
    // Deployment name read by trigger servers and the bootstrap log —
    // carried state, never a process environment variable.
    name: Arc<str>,
    registry: Arc<Registry<StoreCtx<B>>>,
    args: Arc<Vec<String>>,
    mounts: Arc<MountRegistry>,
    // One-shot deployment-supplied HTTP listener, adopted by the HTTP
    // trigger server at boot ([`Runtime::take_http_listener`]).
    http_listener: Mutex<Option<std::net::TcpListener>>,
    // The complete guest environment when the deployment overrides any entry
    // (`HTTP_ADDR` from a supplied listener), merged once at construction;
    // `None` means plain host-environment inheritance.
    guest_env: Option<Arc<Vec<(String, String)>>>,
    backends: B,
    // Resolve-on-miss seam (RFC guest-resolution §4.5). Install-once: hooks
    // ride the deployment builder (or the `from_parts` chainable setters) and
    // never change for the life of the runtime.
    resolver: OnceLock<Arc<dyn GuestResolver>>,
    http_paths: OnceLock<HttpPaths>,
    // Explicit command-mode guest identity; absent, command mode routes to
    // the sole static `wasi:cli/run` exporter.
    command_guest: OnceLock<GuestId>,
    // In-flight resolutions by identity. An entry lives exactly as long as
    // its flight: inserted when the flight starts, removed when its outcome
    // is computed — nothing is cached across flights.
    flights: Mutex<HashMap<GuestId, Flight<B>>>,
}

impl<B: 'static> RuntimeInner<B> {
    fn new(
        name: Arc<str>, registry: Arc<Registry<StoreCtx<B>>>, args: Arc<Vec<String>>,
        mounts: Arc<MountRegistry>, http_listener: Option<std::net::TcpListener>,
        guest_env: Option<Arc<Vec<(String, String)>>>, backends: B,
    ) -> Self {
        Self {
            name,
            registry,
            args,
            mounts,
            http_listener: Mutex::new(http_listener),
            guest_env,
            backends,
            resolver: OnceLock::new(),
            http_paths: OnceLock::new(),
            command_guest: OnceLock::new(),
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
        let name = Arc::<str>::from(deployment.name());
        let args = Arc::new(deployment.args().to_vec());
        link(&mut deployment).context("linking hosts")?;
        let backends = B::connect().await.context("connecting backends")?;
        let mounts = deployment.mounts();
        let (resolver, http_paths) = deployment.resolve_hooks();
        let http_listener = deployment.take_http_listener();
        let command_guest = deployment.command_guest();

        // A supplied listener fixes the guest-visible `HTTP_ADDR` to its
        // actual local address: merge the complete guest environment once,
        // here, so `store()` hands out a ready-made list per store.
        let guest_env = match &http_listener {
            Some(listener) => {
                let addr = listener
                    .local_addr()
                    .context("reading the supplied http listener's local address")?;
                Some(Arc::new(merged_env(&[("HTTP_ADDR".to_owned(), addr.to_string())])))
            }
            None => None,
        };

        let runtime = Self::with_inner(Arc::new(RuntimeInner::new(
            name,
            Arc::new(deployment.into_registry().context("assembling registry")?),
            args,
            mounts,
            http_listener,
            guest_env,
            backends,
        )));
        if let Some(resolver) = resolver {
            runtime.set_resolver(resolver);
        }
        if let Some(hook) = http_paths {
            runtime.set_http_paths(hook);
        }
        if let Some(id) = command_guest {
            runtime.set_command_guest(id);
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
    /// The runtime name defaults to `omnia`.
    #[must_use]
    pub fn from_parts(
        registry: Arc<Registry<StoreCtx<B>>>, args: Vec<String>, mounts: Arc<MountRegistry>,
        backends: B,
    ) -> Self {
        Self::with_inner(Arc::new(RuntimeInner::new(
            Arc::from("omnia"),
            registry,
            Arc::new(args),
            mounts,
            None,
            None,
            backends,
        )))
    }

    /// The deployment name — read by trigger servers and the bootstrap log.
    #[must_use]
    pub fn name(&self) -> &str {
        &self.inner.name
    }

    /// Take the deployment-supplied pre-bound HTTP listener, if any.
    ///
    /// One-shot: the HTTP trigger server adopts it at boot; later calls
    /// return `None` (falling back to the `HTTP_ADDR` environment bind).
    #[must_use]
    pub fn take_http_listener(&self) -> Option<std::net::TcpListener> {
        self.inner.http_listener.lock().unwrap_or_else(PoisonError::into_inner).take()
    }

    /// Install a [`GuestResolver`] consulted on dispatch-path registry misses
    /// (resolve-on-miss), chainable after [`from_parts`](Self::from_parts).
    ///
    /// Deployments built through [`DeploymentBuilder`] supply the resolver via
    /// [`DeploymentBuilder::resolver`] instead. Install-once: a second
    /// resolver is ignored with a warning.
    #[must_use]
    pub fn with_resolver(self, resolver: Arc<dyn GuestResolver>) -> Self {
        self.set_resolver(resolver);
        self
    }

    /// Install an [`HttpPaths`] hook mapping unrouted request paths to guest
    /// identities, chainable after [`from_parts`](Self::from_parts).
    ///
    /// Deployments built through [`DeploymentBuilder`] supply the hook via
    /// [`DeploymentBuilder::http_paths`] instead. Install-once: a second
    /// hook is ignored with a warning.
    #[must_use]
    pub fn with_http_paths<F>(self, hook: F) -> Self
    where
        F: Fn(&str) -> Option<GuestId> + Send + Sync + 'static,
    {
        self.set_http_paths(Arc::new(hook));
        self
    }

    /// How the HTTP trigger routes when its table is empty: a deployment
    /// that installs an [`HttpPaths`] hook owns HTTP routing outright, so
    /// routing is table-driven only — a sole exporter never becomes a
    /// catch-all, and an unmatched path is a miss for the hook to answer.
    #[must_use]
    pub fn http_routing_policy(&self) -> RoutingPolicy {
        if self.inner.http_paths.get().is_some() {
            RoutingPolicy::TableOnly
        } else {
            RoutingPolicy::CapabilityDefault
        }
    }

    /// Build the HTTP trigger's [`TriggerRouter`] over this runtime's
    /// registry, static route table, and [`RoutingPolicy`] — shared by the
    /// trigger server and the testkit harness so the boot-time routing
    /// decision cannot drift between them.
    ///
    /// `probe` resolves a guest's typed handler indices; a guest is capable
    /// exactly when it succeeds.
    ///
    /// # Errors
    ///
    /// Returns an error if a route names a guest that does not export the
    /// handler, or two or more guests export it with no routes under the
    /// capability default.
    pub fn http_trigger_router<I, E, F>(&self, probe: F) -> Result<TriggerRouter<I, HttpRoutes>>
    where
        F: FnMut(&InstancePre<StoreCtx<B>>) -> std::result::Result<I, E>,
    {
        TriggerRouter::build_with(
            self.registry(),
            "http",
            self.registry().routes().http().clone(),
            probe,
            self.http_routing_policy(),
        )
    }

    /// Route command mode to an explicit guest identity, chainable after
    /// [`from_parts`](Self::from_parts).
    ///
    /// Deployments built through [`DeploymentBuilder`] supply the identity via
    /// [`DeploymentBuilder::command_guest`] instead. Install-once: a second
    /// identity is ignored with a warning.
    #[must_use]
    pub fn with_command_guest(self, id: impl Into<GuestId>) -> Self {
        self.set_command_guest(id.into());
        self
    }

    /// The explicit command-mode guest identity, if any.
    #[must_use]
    pub fn command_guest(&self) -> Option<&GuestId> {
        self.inner.command_guest.get()
    }

    fn set_resolver(&self, resolver: Arc<dyn GuestResolver>) {
        if self.inner.resolver.set(resolver).is_err() {
            tracing::warn!("guest resolver already installed; ignoring");
            return;
        }
        // The erased link-path hook holds a weak back-reference: the strong
        // chain RuntimeInner -> Registry -> DispatchHandle -> hook would
        // otherwise cycle.
        self.registry().dispatch().set_resolve_hook(Box::new(RuntimeResolveHook {
            inner: Arc::downgrade(&self.inner),
        }));
    }

    fn set_http_paths(&self, hook: HttpPaths) {
        if self.inner.http_paths.set(hook).is_err() {
            tracing::warn!("http paths hook already installed; ignoring");
        }
    }

    fn set_command_guest(&self, id: GuestId) {
        if self.inner.command_guest.set(id).is_err() {
            tracing::warn!("command guest already installed; ignoring");
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
        StoreCtx {
            base: StoreBase::new(crate::StoreConfig {
                options: self.options(),
                dispatcher: Arc::clone(&self.dispatcher),
                args: Some(Arc::clone(&self.inner.args)),
                mounts: Some(Arc::clone(&self.inner.mounts)),
                env: self.inner.guest_env.as_ref().map(Arc::clone),
            }),
            backends: self.inner.backends.clone(),
        }
    }

    /// Store with epoch deadline, optional fuel, and memory limiter installed.
    ///
    /// # Panics
    ///
    /// Panics if `MAX_FUEL` is set but the engine was built without fuel
    /// metering — a configuration mismatch that would otherwise run guests
    /// unmetered.
    #[must_use]
    pub fn build_store(&self, data: StoreCtx<B>) -> Store<StoreCtx<B>> {
        let options = self.options();
        let mut store = Store::new(self.registry().engine(), data);

        store.set_epoch_deadline(1);
        store.epoch_deadline_async_yield_and_update(1);

        if options.max_fuel > 0 {
            // `Config::from(&options)` enables `consume_fuel` whenever
            // `max_fuel > 0`, so a failure here means the engine was built
            // from different options; running unmetered would silently void
            // the fuel bound.
            store.set_fuel(options.max_fuel).expect("engine was built without fuel metering");
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
        let id = id.into();
        let registry = self.registry();

        // Early occupancy check to skip the load/serve work; the publish below
        // re-checks transactionally, so a racing registration cannot slip in.
        anyhow::ensure!(registry.get(&id).is_none(), "guest `{id}` is already registered");

        let component = artifact
            .load(registry.engine())
            .await
            .with_context(|| format!("loading guest `{id}`"))?;
        self.register_component(id, component).await
    }

    /// [`register`](Self::register) internals over an already-loaded
    /// component — the resolve-on-miss path loads (and export-validates) the
    /// resolver's artifact itself before registering, so validation failure
    /// happens before serve/publish and leaves no partial state.
    async fn register_component(&self, id: GuestId, component: Component) -> Result<()> {
        let registry = self.registry();
        let instance_pre = registry.instantiate_late(&id, &component)?;
        let guest = Guest::local(id.clone(), instance_pre);

        // Wire the guest's linked exports (if any); publish then makes the
        // endpoint and the registry entry observable in one atomic step. If
        // publish refuses (a racing registration won), dropping the unused
        // endpoint aborts its drain tasks.
        let endpoint = serve_guest(self, &guest)
            .await
            .with_context(|| format!("serving guest `{id}` link exports"))?;
        registry.publish(guest, endpoint)?;

        tracing::info!(guest = %id, "guest registered");
        Ok(())
    }

    /// Resolve an unrouted HTTP request path through the deployment's
    /// [`HttpPaths`] hook: path → identity →
    /// [`ensure_guest`](Self::ensure_guest) (and hence resolve-on-miss).
    /// Shared by the HTTP trigger server and the testkit harness so the
    /// outcome semantics cannot drift between them.
    ///
    /// A hook answer of `Some(id)` claims the path, but an identity nothing
    /// supplies ([`EnsureError::Unresolved`] — an unknown tenant) stays
    /// [`RouteRefusal::NotFound`]: an ordinary unmatched request (a 404),
    /// like `None` from the hook or no hook installed. Only a genuine fault
    /// — resolution failed, or the guest lacks the `wasi:http` handler
    /// export — is [`RouteRefusal::Failed`] (a 500 on the wire).
    ///
    /// # Errors
    ///
    /// Returns a [`RouteRefusal`] when no guest can be produced for `path`.
    pub async fn route_http(&self, path: &str) -> Result<Arc<Guest<StoreCtx<B>>>, RouteRefusal> {
        let Some(hook) = self.inner.http_paths.get() else {
            tracing::debug!(path, "no route matched and no http paths hook installed");
            return Err(RouteRefusal::NotFound);
        };
        let Some(target) = hook(path) else {
            tracing::debug!(path, "no route matched and the http paths hook declined the path");
            return Err(RouteRefusal::NotFound);
        };
        match self.ensure_guest(&target, "wasi:http/handler").await {
            Ok(guest) => Ok(guest),
            Err(EnsureError::Unresolved(_)) => {
                tracing::warn!(path, guest = %target, "claimed path names a guest nothing supplies");
                Err(RouteRefusal::NotFound)
            }
            Err(error) => Err(RouteRefusal::Failed(error)),
        }
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
    /// registration failed, and [`EnsureError::ExportMismatch`] when the
    /// resolved (or concurrently registered) component lacks
    /// `expected_export`.
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

    /// The number of dispatches currently sharing the resolve-on-miss flight
    /// for `id` (zero when none is active) — a seam-suite probe, so
    /// single-flight tests gate on joined waiters instead of sleeping.
    #[doc(hidden)]
    #[must_use]
    pub fn flight_waiters(&self, id: &GuestId) -> usize {
        let flights = self.inner.flights.lock().unwrap_or_else(PoisonError::into_inner);
        // The map itself holds one clone of the shared future; the rest are
        // waiters. `strong_count` is `None` once the future has completed.
        flights
            .get(id)
            .and_then(futures::future::Shared::strong_count)
            .map_or(0, |count| count.saturating_sub(1))
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

    /// Release every link-serve endpoint, aborting the drain tasks that pin
    /// `Runtime` clones (and with them the engine's pooling reservation).
    ///
    /// [`run`] does this as the drive completes; an embedder holding a
    /// [`from_parts`](Self::from_parts) runtime calls it when the deployment
    /// is finished. In-flight invocations hold their own server handles and
    /// complete; only new dispatches are cut off.
    pub fn shutdown(&self) {
        self.registry().dispatch().transport().clear();
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

    // A resolver's answer is not trusted to be well-shaped: load and validate
    // the component against the dispatch site's required export before any
    // registration, so the mismatch is typed and nothing is published.
    let component = artifact.load(runtime.registry().engine()).await.map_err(|error| {
        let error = error.context(format!("loading resolved guest `{id}`"));
        tracing::error!(guest = %id, "guest resolution failed: {error:#}");
        EnsureError::ResolveFailed(Arc::new(error))
    })?;
    if !exports_instance(&component, runtime.registry().engine(), expected_export) {
        return Err(EnsureError::ExportMismatch {
            guest: id.clone(),
            export: expected_export.to_owned(),
        });
    }

    let raced = match runtime.register_component(id.clone(), component).await {
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
        assert_eq!(ExitStatus::from(256).code_u8(), 0);
        assert_eq!(ExitStatus::from(257).code_u8(), 1);
        assert_eq!(ExitStatus::from(-1).code_u8(), 255);
    }
}
