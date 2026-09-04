//! Deployment lifecycle: [`Backends`], [`Wiring`], [`run`], and [`run_with`].

use std::future::Future;
use std::process::ExitCode;
use std::time::Duration;

use anyhow::{Context as _, Result};
use omnia_core::wasmtime::Engine;
use omnia_core::{ExitStatus, Runtime, StoreCtx};

use crate::{Deployment, DeploymentBuilder};

/// A deployment's connected backend bundle, threaded into [`Runtime`].
///
/// The `runtime!` macro generates the concrete bundle (one field per declared
/// backend) and this impl, whose [`connect`](Self::connect) connects every
/// backend concurrently. A deployment that wires no backends uses the
/// [`()`](unit) bundle below, so [`Runtime`] needs no special empty case.
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
///
/// Bounded on the bundle's shape rather than on [`Backends`] so one wiring
/// serves both the connected production bundle and a bundle handed in ready
/// (see [`run_with`]).
pub trait Wiring<B: Clone + Send + Sync + 'static> {
    /// Link every declared host into the deployment linker.
    ///
    /// # Errors
    ///
    /// Returns an error if a host cannot be added to the linker.
    fn link(deployment: &mut Deployment<StoreCtx<B>>) -> Result<()>;

    /// Install capability extensions into [`Runtime::extensions`]. Invoked
    /// once, after the bundle is in hand and the runtime is assembled, so an
    /// extension is built against the bundle (via [`Runtime::backends`]);
    /// the default installs nothing.
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

/// Run a planned deployment builder to completion, reporting failures on stderr.
pub async fn drive_main<B, H>(builder: DeploymentBuilder) -> ExitCode
where
    B: Backends,
    H: Wiring<B>,
{
    // The generated entry point admits pre-compiled artifacts: manifests and
    // `.bin` paths given to (or compiled into) the binary are trusted
    // operator inputs (docs/security-model.md).
    match async {
        // SAFETY: the operator running this binary chose the manifest and
        // artifact paths; pre-compiled artifacts are documented trusted inputs
        // produced by `omnia compile`.
        let deployment =
            unsafe { builder.build_trusted::<StoreCtx<B>>() }.await.context("building runtime")?;
        run::<B, H>(deployment).await
    }
    .await
    {
        Ok(status) => status.into(),
        Err(error) => {
            eprintln!("{error:#}");
            ExitCode::FAILURE
        }
    }
}

/// Connect backends, bootstrap the runtime, then run command mode or every trigger server.
///
/// # Errors
///
/// Returns an error if backends cannot connect, runtime state cannot be
/// assembled, bootstrap fails, or a trigger server exits with an error.
pub async fn run<B, H>(deployment: Deployment<StoreCtx<B>>) -> Result<ExitStatus>
where
    B: Backends,
    H: Wiring<B>,
{
    let backends = B::connect().await.context("connecting backends")?;
    run_with::<B, H>(deployment, backends).await
}

/// [`run`] over a bundle already in hand: nothing connects, `backends` is
/// threaded in as-is. The shape a test harness uses to drive a production
/// `runtime!`'s wiring over in-memory backends.
///
/// # Errors
///
/// Returns an error if runtime state cannot be assembled, bootstrap fails, or
/// a trigger server exits with an error.
pub async fn run_with<B, H>(
    mut deployment: Deployment<StoreCtx<B>>, backends: B,
) -> Result<ExitStatus>
where
    B: Clone + Send + Sync + 'static,
    H: Wiring<B>,
{
    let mode = deployment.mode();
    H::link(&mut deployment).context("linking hosts")?;
    let runtime = deployment.assemble(backends).await.context("assembling runtime")?;
    H::extend(&runtime).context("installing runtime extensions")?;
    finish::<B, H>(runtime, mode).await
}

/// Start background tasks, then run command mode or every trigger server,
/// releasing the runtime when the drive completes.
async fn finish<B, H>(runtime: Runtime<B>, mode: Mode) -> Result<ExitStatus>
where
    B: Clone + Send + Sync + 'static,
    H: Wiring<B>,
{
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
            runtime.run_command().await
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
    omnia_core::telemetry::flush();
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
