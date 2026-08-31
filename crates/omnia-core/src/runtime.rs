//! Deployment lifecycle: [`Backends`], [`Wiring`], [`Runtime`], [`run`], and [`ExitStatus`].

mod command;
mod entry;

use std::future::Future;
use std::process::ExitCode;
use std::sync::{Arc, OnceLock, Weak};
use std::time::Duration;
use std::{env, fmt};

use anyhow::{Context as _, Result};
pub use entry::{MainOptions, ManifestSource};
use wasmtime::component::{Component, Instance, InstancePre, types};
use wasmtime::{Engine, Store};

use crate::deployment::{ELF_MAGIC, GuestArtifact};
use crate::dispatch::{serve_guest, serve_links};
use crate::extensions::Extensions;
use crate::mount::MountRegistry;
use crate::registry::{Guest, GuestId, HttpRoutes, TriggerRouter};
use crate::store::HasLimits;
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
    fn connect() -> impl Future<Output = Result<Self>> {
        std::future::ready(Ok(()))
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

/// Host linking, extension installation, and trigger-server startup for a
/// deployment.
pub trait Wiring<B: Backends> {
    /// Link every declared host into the deployment linker.
    ///
    /// # Errors
    ///
    /// Returns an error if a host cannot be added to the linker.
    fn link(deployment: &mut Deployment<StoreCtx<B>>) -> Result<()>;

    /// Install capability extensions into [`Runtime::extensions`]. Invoked by
    /// [`Runtime::new`] once, after [`Backends::connect`] and runtime
    /// assembly, so an extension is built against the connected bundle (via
    /// [`Runtime::backends`]); the default installs nothing.
    ///
    /// # Errors
    ///
    /// Returns an error if an extension cannot be built or installed.
    fn extend(runtime: &Runtime<B>) -> Result<()> {
        let _ = runtime;
        Ok(())
    }

    /// Run every declared long-lived trigger server concurrently.
    fn serve(runtime: &Runtime<B>) -> impl std::future::Future<Output = Result<()>> + Send;
}

/// Entry point for generated `main` functions.
///
/// `options` carries the deployment the `runtime!` macro compiled in: mode
/// and manifest source. Command mode with a compiled-in deployment is a
/// direct command: argv passes to the guest verbatim except the reserved host
/// log flags (`--debug` / `--quiet`), which select the telemetry
/// [`LogMode`](crate::LogMode). Every other shape parses the standard
/// `run [wasm] [--config] -- args…` grammar.
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

    let runtime =
        Runtime::<B>::new(deployment, H::link, H::extend).await.context("assembling runtime")?;

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
    if mode.is_command() {
        tracing::debug!(component = runtime.name(), "omnia ready");
    } else {
        tracing::info!(component = runtime.name(), "omnia ready");
    }
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

struct RuntimeInner<B: 'static> {
    // Deployment name read by trigger servers and the bootstrap log —
    // carried state, never a process environment variable.
    name: Arc<str>,
    registry: Arc<Registry<StoreCtx<B>>>,
    args: Arc<Vec<String>>,
    mounts: Arc<MountRegistry>,
    backends: B,
    // Manifest-marked command guest identity; absent, command mode routes to
    // the sole static `wasi:cli/run` exporter.
    command_guest: OnceLock<GuestId>,
    // Capability-crate state installed by the `Wiring::extend` hook and
    // shared with every store context.
    extensions: Extensions,
}

impl<B: 'static> RuntimeInner<B> {
    fn new(
        name: Arc<str>, registry: Arc<Registry<StoreCtx<B>>>, args: Arc<Vec<String>>,
        mounts: Arc<MountRegistry>, backends: B,
    ) -> Self {
        Self {
            name,
            registry,
            args,
            mounts,
            backends,
            command_guest: OnceLock::new(),
            extensions: Extensions::new(),
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

/// A non-owning [`Runtime`] handle, the form a runtime extension holds to
/// call back into the runtime without leaking it through a reference cycle.
pub struct WeakRuntime<B: 'static> {
    inner: Weak<RuntimeInner<B>>,
}

// Manual: a handle clone must not require `B: Clone`.
impl<B: 'static> Clone for WeakRuntime<B> {
    fn clone(&self) -> Self {
        Self {
            inner: Weak::clone(&self.inner),
        }
    }
}

impl<B: Clone + Send + Sync + 'static> WeakRuntime<B> {
    /// Upgrade to a full handle; `None` once the runtime has shut down.
    #[must_use]
    pub fn upgrade(&self) -> Option<Runtime<B>> {
        Some(Runtime::with_inner(self.inner.upgrade()?))
    }
}

/// Why [`Runtime::admit`] refused a late guest; each variant carries the
/// refusal's description.
#[derive(Clone, Debug)]
pub enum AdmitError {
    /// The bytes are a native artifact, not a valid raw wasm component, or
    /// failed pre-instantiation against the deployment's host set.
    ArtifactRefused(String),
    /// The component exports no interface declared in the deployment's
    /// plugin seam list.
    SeamMissing(String),
    /// Serve wiring or publication failed — including an identity conflict
    /// with a racing registration.
    Internal(String),
}

impl fmt::Display for AdmitError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::ArtifactRefused(reason) | Self::SeamMissing(reason) | Self::Internal(reason) => {
                f.write_str(reason)
            }
        }
    }
}

impl std::error::Error for AdmitError {}

impl<B: Backends> Runtime<B> {
    /// Link hosts, connect backends, assemble the guest registry, install
    /// capability extensions, and wire the host-mediated link serve side.
    ///
    /// `extend` is the [`Wiring::extend`] hook: invoked once, after backends
    /// connect and the runtime is assembled, so an extension is built against
    /// the connected bundle.
    ///
    /// # Errors
    ///
    /// Returns an error if host linking, backend connection, registry
    /// assembly, extension installation, or link serve wiring fails.
    pub async fn new<L, E>(
        mut deployment: Deployment<StoreCtx<B>>, link: L, extend: E,
    ) -> Result<Self>
    where
        L: FnOnce(&mut Deployment<StoreCtx<B>>) -> Result<()>,
        E: FnOnce(&Self) -> Result<()>,
    {
        let name = Arc::<str>::from(deployment.name());
        let args = Arc::new(deployment.args().to_vec());
        link(&mut deployment).context("linking hosts")?;
        let backends = B::connect().await.context("connecting backends")?;
        let mounts = deployment.mounts();
        let command_guest = deployment.command_guest();

        let runtime = Self::with_inner(Arc::new(RuntimeInner::new(
            name,
            Arc::new(deployment.into_registry().context("assembling registry")?),
            args,
            mounts,
            backends,
        )));
        if let Some(id) = command_guest {
            runtime.set_command_guest(id);
        }
        extend(&runtime).context("installing runtime extensions")?;
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
    /// `plugins` interfaces must run [`serve_links`] itself before dispatching.
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
            backends,
        )))
    }

    /// The deployment name — read by trigger servers and the bootstrap log.
    #[must_use]
    pub fn name(&self) -> &str {
        &self.inner.name
    }

    /// Build the HTTP trigger's [`TriggerRouter`] over this runtime's
    /// registry and static route table so the boot-time routing decision
    /// lives in one place.
    ///
    /// `probe` resolves a guest's typed handler indices; a guest is capable
    /// exactly when it succeeds.
    ///
    /// # Errors
    ///
    /// Returns an error if a route names a guest that does not export the
    /// handler, or two or more guests export it with no routes.
    pub fn http_trigger_router<I, E, F>(&self, probe: F) -> Result<TriggerRouter<I, HttpRoutes>>
    where
        F: FnMut(&InstancePre<StoreCtx<B>>) -> std::result::Result<I, E>,
    {
        TriggerRouter::build(
            self.registry(),
            "http",
            self.registry().routes().http().clone(),
            probe,
        )
    }

    /// The command-mode guest identity (the manifest entry marked
    /// `command = true`), if any.
    #[must_use]
    pub fn command_guest(&self) -> Option<&GuestId> {
        self.inner.command_guest.get()
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

    /// The deployment's connected backend bundle.
    #[must_use]
    pub fn backends(&self) -> &B {
        &self.inner.backends
    }

    /// The capability-crate state installed by the [`Wiring::extend`] hook —
    /// the same set every store context carries.
    #[must_use]
    pub fn extensions(&self) -> &Extensions {
        &self.inner.extensions
    }

    /// A non-owning handle for state that must call back into the runtime;
    /// see [`Extensions`] for why extensions never hold a [`Runtime`].
    #[must_use]
    pub fn downgrade(&self) -> WeakRuntime<B> {
        WeakRuntime {
            inner: Arc::downgrade(&self.inner),
        }
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
                env: None,
                extensions: self.inner.extensions.clone(),
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

    /// Drive the deployment's `wasi:cli/run` command once, returning the
    /// guest's exit status.
    ///
    /// # Errors
    ///
    /// Returns an error if the command guest is not registered, routing is
    /// ambiguous, the guest cannot be instantiated, or the command traps
    /// without a guest exit code.
    pub async fn run_command(&self) -> Result<ExitStatus> {
        command::drive(self).await
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
    /// and `plugins` set, or its linked exports cannot be served.
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
    /// component.
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

        tracing::debug!(guest = %id, "guest registered");
        Ok(())
    }

    /// Admit raw wasm bytes as a late guest: refuse a native (pre-compiled)
    /// artifact before wasmtime sees the bytes, validate on the safe path,
    /// require an export of a declared plugin interface, then register and
    /// serve the component under `id` — the privileged registration half
    /// behind the `omnia:plugins/loader` capability. Acquisition, digest
    /// policy, and idempotency live with the loader (`omnia-plugin`).
    ///
    /// # Errors
    ///
    /// Returns a typed [`AdmitError`] naming the refusal: refused artifact,
    /// missing seam export, or an internal serve/publication failure
    /// (including an identity conflict with a racing registration).
    pub async fn admit(&self, id: GuestId, bytes: Vec<u8>) -> Result<(), AdmitError> {
        // A native (pre-compiled) artifact is refused before wasmtime sees
        // the bytes: admitted components only ever take the safe validation
        // path, never the deployment's `Precompiled` trust policy.
        if bytes.get(..ELF_MAGIC.len()) == Some(&ELF_MAGIC) {
            return Err(AdmitError::ArtifactRefused(format!(
                "`{id}` is a pre-compiled (native) artifact; admission only accepts raw wasm \
                 components"
            )));
        }

        // Safe validation plus sandboxed JIT — the explicitly safe constructor.
        let component =
            GuestArtifact::wasm(bytes).load(self.registry().engine()).await.map_err(|error| {
                AdmitError::ArtifactRefused(format!("validating `{id}`: {error:#}"))
            })?;

        self.check_seam_export(&id, &component)?;

        // The same publish sequence as `Runtime::register`: pre-instantiate
        // against the shared host set, wire seam exports, publish atomically.
        let instance_pre = self.registry().instantiate_late(&id, &component).map_err(|error| {
            AdmitError::ArtifactRefused(format!("pre-instantiating `{id}`: {error:#}"))
        })?;
        let guest = Guest::local(id.clone(), instance_pre);
        let endpoint = serve_guest(self, &guest).await.map_err(|error| {
            AdmitError::Internal(format!("serving `{id}` seam exports: {error:#}"))
        })?;
        self.registry()
            .publish(guest, endpoint)
            .map_err(|error| AdmitError::Internal(format!("publishing `{id}`: {error:#}")))?;

        tracing::debug!(guest = %id, "late guest admitted");
        Ok(())
    }

    /// Refuse a component that exports no interface from the deployment's
    /// declared plugin seam list.
    fn check_seam_export(&self, id: &GuestId, component: &Component) -> Result<(), AdmitError> {
        let links = self.registry().dispatch().links();
        if links.is_empty() {
            return Err(AdmitError::SeamMissing(format!(
                "cannot admit `{id}`: this deployment declares no plugin interfaces"
            )));
        }
        let engine = self.registry().engine();
        let exports_seam =
            component.component_type().exports(engine).any(|(interface, extern_)| {
                links.contains(interface)
                    && matches!(extern_.ty, types::ComponentItem::ComponentInstance(_))
            });
        if exports_seam {
            Ok(())
        } else {
            let declared: Vec<&str> = links.iter().map(AsRef::as_ref).collect();
            Err(AdmitError::SeamMissing(format!(
                "`{id}` exports none of the declared plugin interfaces ({})",
                declared.join(", ")
            )))
        }
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
        tracing::debug!(guest = %id, "guest deregistered");
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
