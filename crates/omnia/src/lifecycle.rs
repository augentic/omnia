//! Deployment lifecycle: [`Backends`], [`Wiring`], [`run`], and [`run_with`].

use std::future::Future;
use std::process::ExitCode;
use std::time::{Duration, Instant};

use anyhow::{Context as _, Result};
use omnia_core::wasmtime::Engine;
use omnia_core::{ExitStatus, LevelFilter, Runtime, StoreCtx, WeakRuntime};

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

// the zero-backend bundle: a deployment linking only backend-less hosts connects nothing
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

    /// The tracing level a run in this mode falls back to when no level is
    /// selected and the process sets no `RUST_LOG`.
    ///
    /// A command shows its progress, so [`Command`](Self::Command) is `INFO`;
    /// a server is quiet until something is wrong, so [`Server`](Self::Server)
    /// is `WARN`. The verbosity flags step from here, one rung per `-v` up and
    /// per `-q` down.
    #[must_use]
    pub const fn level(self) -> LevelFilter {
        match self {
            Self::Server => LevelFilter::WARN,
            Self::Command => LevelFilter::INFO,
        }
    }
}

/// Host linking and trigger-server startup for a deployment.
///
/// Bounded on the bundle's shape rather than on [`Backends`] so one wiring
/// serves both the connected production bundle and a bundle handed in ready
/// (see [`run_with`]). The guest loader is not wired here: assembly links
/// its host and installs its policy for every deployment alike.
pub trait Wiring<B: Clone + Send + Sync + 'static> {
    /// Link every declared host into the deployment linker.
    ///
    /// # Errors
    ///
    /// Returns an error if a host cannot be added to the linker.
    fn link(deployment: &mut Deployment<StoreCtx<B>>) -> Result<()>;

    /// Run every declared long-lived trigger server concurrently.
    fn serve(runtime: &Runtime<B>) -> impl std::future::Future<Output = Result<()>> + Send;
}

/// Run a planned deployment builder to completion, reporting failures on stderr.
pub async fn drive_main<B, H>(builder: DeploymentBuilder) -> ExitCode
where
    B: Backends,
    H: Wiring<B>,
{
    match async {
        let deployment = builder.build::<StoreCtx<B>>().await.context("building runtime")?;
        run::<B, H>(deployment).await
    }
    .await
    {
        Ok(status) => status.into(),
        Err(error) => report(&error),
    }
}

// The fatal-error path of a generated `main`: the deployment is not up, or
// never came up, so stderr is the one channel a subscriber may not be on.
#[expect(clippy::print_stderr, reason = "the fatal-error channel, with or without a subscriber")]
pub fn report(error: &anyhow::Error) -> ExitCode {
    eprintln!("{error:#}");
    ExitCode::FAILURE
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
    finish::<B, H>(runtime, mode).await
}

async fn finish<B, H>(runtime: Runtime<B>, mode: Mode) -> Result<ExitStatus>
where
    B: Clone + Send + Sync + 'static,
    H: Wiring<B>,
{
    // background tasks hold engine clones; aborted after the drive so a
    // finished deployment releases its pooling reservation
    let engine = runtime.registry().engine();
    let options = runtime.options();
    let epoch = drive_epoch(engine.clone(), options.epoch_tick);
    let metrics = sample_pool(engine.clone(), options.pool_metrics_interval);
    let idle = release_idle_pool(runtime.downgrade(), options.pool_idle_release);

    tracing::info!(component = runtime.name(), "omnia ready");

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
    if let Some(metrics) = metrics {
        metrics.abort();
    }
    if let Some(idle) = idle {
        idle.abort();
    }

    // the link-serve drain tasks hold runtime clones that would pin the engine
    runtime.shutdown();

    // batch-queued telemetry must survive a fast command-mode exit
    omnia_core::telemetry::flush();
    outcome
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

            tracing::trace!(
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

// holds no engine clone: the runtime is reached through the weak handle each
// look, so the task ends with the deployment rather than pinning its pool
fn release_idle_pool<B>(
    runtime: WeakRuntime<B>, after: Duration,
) -> Option<tokio::task::JoinHandle<()>>
where
    B: Clone + Send + Sync + 'static,
{
    if after.is_zero() {
        return None;
    }

    let handle = tokio::spawn(async move {
        let mut idle = IdleRelease::new(after);
        let mut ticker = tokio::time::interval(idle.tick());
        loop {
            ticker.tick().await;

            let Some(runtime) = runtime.upgrade() else {
                break;
            };
            let engine = runtime.registry().engine();
            let Some(metrics) = engine.pooling_allocator_metrics() else {
                break;
            };

            // live first, then the count: a store built between the two reads
            // changes the count, so no invocation falls between a look's reads
            let live = metrics.component_instances() > 0 || metrics.core_instances() > 0;
            if !idle.due(runtime.stores_built(), live, Instant::now()) {
                continue;
            }

            // a release already made leaves nothing resident until the pool is busy again
            if metrics.unused_memory_bytes_resident() == 0 {
                continue;
            }
            let bytes = engine.release_idle_pool_memory();
            tracing::debug!(bytes, "released idle pool memory");
        }
    });
    Some(handle)
}

// when warm slots' resident memory goes back to the OS: once no guest has
// been built or live for `after`, judged from the runtime's store count rather
// than a live gauge alone, so an invocation that begins and ends between two
// looks still counts as activity
struct IdleRelease {
    after: Duration,
    seen: u64,
    idle_since: Option<Instant>,
}

impl IdleRelease {
    // the longest a look waits, so a release lands within this of `after`
    const TICK: Duration = Duration::from_secs(1);

    const fn new(after: Duration) -> Self {
        Self {
            after,
            seen: 0,
            idle_since: None,
        }
    }

    fn tick(&self) -> Duration {
        self.after.min(Self::TICK)
    }

    // whether the pool has now been idle for `after`: `built` is the runtime's
    // store count, `live` whether any instance is allocated; a true answer
    // ends the idle spell, so one spell releases once
    fn due(&mut self, built: u64, live: bool, now: Instant) -> bool {
        if live {
            self.seen = built;
            self.idle_since = None;
            return false;
        }
        if built != self.seen {
            self.seen = built;
            self.idle_since = Some(now);
            return false;
        }

        let since = *self.idle_since.get_or_insert(now);
        if now.duration_since(since) < self.after {
            return false;
        }
        self.idle_since = None;
        true
    }
}

#[cfg(test)]
mod tests {
    use std::time::{Duration, Instant};

    use super::IdleRelease;

    const AFTER: Duration = Duration::from_secs(10);
    // one tick short of `AFTER`
    const ALMOST: Duration = Duration::from_millis(9_999);

    #[test]
    fn activity_between_ticks() {
        let mut idle = IdleRelease::new(AFTER);
        let start = Instant::now();
        assert!(!idle.due(1, false, start));
        assert!(!idle.due(1, false, start + AFTER / 2));

        // a guest ran and finished between two looks, each of which saw no
        // live instance: the spell restarts at the look that saw the count move
        assert!(!idle.due(2, false, start + AFTER));
        assert!(!idle.due(2, false, start + AFTER + ALMOST));
        assert!(idle.due(2, false, start + 2 * AFTER));
    }

    #[test]
    fn live_instance() {
        let mut idle = IdleRelease::new(AFTER);
        let start = Instant::now();
        assert!(!idle.due(0, false, start));
        assert!(!idle.due(0, true, start + AFTER / 2));

        // idle begins at the first look after the instance is gone
        assert!(!idle.due(0, false, start + AFTER));
        assert!(!idle.due(0, false, start + AFTER + ALMOST));
        assert!(idle.due(0, false, start + 2 * AFTER));
    }

    #[test]
    fn idle_short_of_after() {
        let mut idle = IdleRelease::new(AFTER);
        let start = Instant::now();
        assert!(!idle.due(0, false, start));
        assert!(!idle.due(0, false, start + ALMOST));
    }

    #[test]
    fn idle_past_after() {
        let mut idle = IdleRelease::new(AFTER);
        let start = Instant::now();
        assert!(!idle.due(0, false, start));
        assert!(idle.due(0, false, start + AFTER));

        // the spell that released is over; the next waits `after` again
        assert!(!idle.due(0, false, start + AFTER));
        assert!(idle.due(0, false, start + 2 * AFTER));
    }

    #[test]
    fn tick_within_after() {
        assert_eq!(IdleRelease::new(AFTER).tick(), IdleRelease::TICK);
        let short = Duration::from_millis(200);
        assert_eq!(IdleRelease::new(short).tick(), short);
    }
}
