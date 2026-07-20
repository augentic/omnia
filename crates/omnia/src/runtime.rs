//! Deployment lifecycle: [`Backends`], [`Wiring`], [`Runtime`], [`run`], and [`ExitStatus`].

mod command;

use std::env;
use std::future::Future;
use std::process::ExitCode;
use std::sync::Arc;
use std::time::Duration;

use anyhow::{Context as _, Result};
use clap::Parser as _;
use wasmtime::component::{Instance, InstancePre};
use wasmtime::{Engine, Store};

use crate::cli::{Cli, Command};
use crate::dispatch::serve_links;
use crate::mount::MountRegistry;
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

/// CLI entry point for generated `main` functions.
///
/// `default_config` is the runtime's compiled-in manifest fallback (the
/// `runtime!` macro's `config:` field), used only when the CLI supplies no
/// source.
#[doc(hidden)]
pub async fn main<B, H>(mode: Mode, default_config: Option<std::path::PathBuf>) -> ExitCode
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
                                default_config
                                    .context(
                                        "no guest specified: pass a <wasm> path, or --config \
                                         <omnia.toml> (or set OMNIA_CONFIG)",
                                    )
                                    .and_then(Manifest::from_config)
                            },
                            |wasm| Ok(Manifest::from_wasm(wasm)),
                        )
                    },
                    Manifest::from_config,
                )
                .map(|manifest| manifest.mounts(mounts).links(links));

            let result = match manifest {
                Ok(manifest) => {
                    let builder = DeploymentBuilder::new().manifest(manifest).args(args).mode(mode);
                    run::<B, H>(builder).await
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
    let mode = deployment.mode();

    let runtime = Runtime::<B>::new(deployment, H::link).await.context("assembling runtime")?;

    // start background tasks
    drive_epoch(runtime.registry().engine().clone(), runtime.options().epoch_tick);
    sample_pool(runtime.registry().engine().clone(), runtime.options().pool_metrics_interval);

    // wire host-mediated link servers
    serve_links(&runtime).await.context("wiring host-mediated link serve side")?;

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
        guests = runtime.registry().guests().count(),
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

struct RuntimeInner<B: 'static> {
    registry: Arc<Registry<StoreCtx<B>>>,
    args: Arc<Vec<String>>,
    mounts: Arc<MountRegistry>,
    backends: B,
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
    /// Link hosts, connect backends, and assemble the guest registry.
    ///
    /// # Errors
    ///
    /// Returns an error if host linking, backend connection, or registry assembly fails.
    pub async fn new<L>(mut deployment: Deployment<StoreCtx<B>>, link: L) -> Result<Self>
    where
        L: FnOnce(&mut Deployment<StoreCtx<B>>) -> Result<()>,
    {
        let args = Arc::new(deployment.args().to_vec());
        link(&mut deployment).context("linking hosts")?;
        let backends = B::connect().await.context("connecting backends")?;
        let mounts = deployment.mounts();

        Ok(Self::with_inner(Arc::new(RuntimeInner {
            registry: Arc::new(deployment.into_registry().context("assembling registry")?),
            args,
            mounts,
            backends,
        })))
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
    #[must_use]
    pub fn from_parts(
        registry: Arc<Registry<StoreCtx<B>>>, args: Vec<String>, mounts: Arc<MountRegistry>,
        backends: B,
    ) -> Self {
        Self::with_inner(Arc::new(RuntimeInner {
            registry,
            args: Arc::new(args),
            mounts,
            backends,
        }))
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
