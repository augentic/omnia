//! # Runtime lifecycle
//!
//! The startup [`prepare`] every deployment shares and the long-lived server
//! loop driven by [`run`], plus the detached background tasks they drive off the
//! Wasmtime [`Engine`] (epoch interruption so guest deadlines fire while
//! CPU-bound guests execute, and pooling-allocator occupancy sampling emitted
//! as `OpenTelemetry` gauges via the `tracing` metrics bridge) and the
//! [`ExitStatus`] a deployment yields.

use std::future::Future;
use std::path::PathBuf;
use std::process::ExitCode;
use std::sync::Arc;
use std::time::Duration;

use anyhow::{Context as _, Result};
use clap::Parser as _;
use futures::future::{self, BoxFuture};
use wasmtime::Engine;
use wasmtime_wasi::WasiView;
use wrpc_wasmtime::WrpcView;

use crate::dispatch::serve_links;
use crate::traits::{Backends, BuildStore, HasLimits, Runtime};
use crate::working_tree::WorkingTreeRegistry;
use crate::{Cli, Command, Compiled, Registry, RegistryBuilder, StoreBase, command};

/// The standard host [`Runtime`] the `runtime!` macro builds for a deployment.
///
/// It owns the fixed runtime state every deployment shares — the guest
/// [`Registry`], the guest argv, and the working-tree registry — plus the
/// deployment's connected [`Backends`] bundle `B`. The per-store context type
/// `S` (the `StoreCtx`) stays in the deployment crate, because its host-view
/// impls are foreign-trait impls bound to that local type; `Context` reaches it
/// only through the [`BuildStore`] seam.
///
/// The macro previously emitted this struct, its [`new`](Self::new), and a
/// derived [`Runtime`] impl inline; hosting it here keeps that boilerplate (and
/// the backend-connection lifecycle) in the library.
pub struct Context<S: 'static, B> {
    registry: Arc<Registry<S>>,
    args: Arc<Vec<String>>,
    working_trees: Arc<WorkingTreeRegistry>,
    backends: B,
}

// A derived `Clone` would demand `S: Clone`, but the `StoreCtx` is never `Clone`
// (it owns the WASI table); every field here is either shared behind an `Arc` or
// comes from the `Clone` bundle, so only `B: Clone` is required.
impl<S: 'static, B: Clone> Clone for Context<S, B> {
    fn clone(&self) -> Self {
        Self {
            registry: Arc::clone(&self.registry),
            args: Arc::clone(&self.args),
            working_trees: Arc::clone(&self.working_trees),
            backends: self.backends.clone(),
        }
    }
}

impl<S, B> Context<S, B>
where
    S: WasiView + WrpcView + HasLimits + BuildStore<B> + Send + 'static,
    B: Backends,
{
    /// Assemble the runtime from a [`Compiled`] deployment: thread the guest
    /// argv, link the deployment's hosts (via `link`), connect every backend,
    /// and freeze the guest [`Registry`].
    ///
    /// `link` is the one per-deployment step the macro still supplies — a
    /// closure that calls [`Compiled::host`] once per wired host (the generated
    /// `servers` closure mirrors it when starting trigger servers).
    ///
    /// # Errors
    ///
    /// Returns an error if host linking, backend connection, or registry
    /// assembly fails.
    pub async fn new<L>(mut compiled: Compiled<S>, link: L) -> Result<Self>
    where
        L: FnOnce(&mut Compiled<S>) -> Result<()>,
    {
        let args = Arc::new(compiled.args().to_vec());
        link(&mut compiled).context("linking hosts")?;
        let backends = B::connect().await.context("connecting backends")?;
        let working_trees = compiled.working_trees();

        Ok(Self {
            registry: Arc::new(compiled.build().context("assembling registry")?),
            args,
            working_trees,
            backends,
        })
    }
}

impl<S, B> Runtime for Context<S, B>
where
    S: WasiView + WrpcView + HasLimits + BuildStore<B> + Send + 'static,
    B: Backends,
{
    type StoreCtx = S;

    fn registry(&self) -> &Registry<Self::StoreCtx> {
        &self.registry
    }

    fn store(&self) -> Self::StoreCtx {
        // `HostDispatch` is blanket-implemented for every `Runtime`, so a fresh
        // clone backs any host->guest call; the generated `BuildStore` impl then
        // clones each wired backend into its host-view field.
        let base = StoreBase::builder()
            .options(self.options())
            .dispatch(Arc::new(self.clone()))
            .args(&self.args)
            .working_trees(Arc::clone(&self.working_trees))
            .build();
        S::build_store(base, &self.backends)
    }
}

/// Spawn a detached background task that drives epoch interruption.
///
/// Calls [`Engine::increment_epoch`] every `tick`. Together with the per-store
/// epoch deadline installed in `Runtime::build_store`, this is what lets a
/// CPU-bound guest periodically yield to the async executor so the wall-clock
/// timeout wrapped around each invocation can fire.
///
/// `tick` must be non-zero: the runtime clamps `EPOCH_TICK_MS` to a 1ms minimum
/// (see `parse_tick`), and [`tokio::time::interval`] panics on a zero period.
pub fn drive_epoch(engine: Engine, tick: Duration) {
    tokio::spawn(async move {
        let mut interval = tokio::time::interval(tick);
        loop {
            interval.tick().await;
            engine.increment_epoch();
        }
    });
}

/// Spawn a detached background task that samples pool occupancy as metrics.
///
/// Periodically reads `engine`'s pooling-allocator occupancy and emits it as
/// `OpenTelemetry` gauges (through the `tracing` -> `OpenTelemetry` metrics
/// layer configured by [`Telemetry`]).
///
/// The task is a no-op and is never spawned when `interval` is zero. If the
/// engine was not configured with the pooling allocator (so there are no pool
/// metrics to report) the task stops after its first tick.
pub fn sample_pool(engine: Engine, interval: Duration) {
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

            tracing::info!(
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

/// Parse the CLI `run` subcommand and map the outcome to a process exit code.
///
/// Generated `main` functions delegate here so CLI parsing and error reporting
/// stay in the library rather than in proc-macro output.
///
/// # Errors
///
/// Failures from `run` are printed to stderr and mapped to
/// [`ExitCode::FAILURE`]; success yields the guest or deployment status.
#[doc(hidden)]
pub async fn main<R, N, NFut, S>(command_mode: bool, new: N, servers: S) -> ExitCode
where
    R: Runtime,
    N: FnOnce(Compiled<R::StoreCtx>) -> NFut,
    NFut: Future<Output = Result<R>>,
    S: for<'a> FnOnce(&'a R) -> Vec<BoxFuture<'a, Result<()>>>,
{
    match Cli::parse().command {
        Command::Run { wasm, config, args } => {
            match run(wasm, config, args, command_mode, new, servers).await {
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

/// Drive a deployment after runtime state is built: a one-shot `wasi:cli` command
/// or every long-lived trigger server to completion.
///
/// # Errors
///
/// Returns an error if preparation, command execution, or any server fails.
pub async fn run<R, N, NFut, S>(
    wasm: Option<PathBuf>, config: Option<PathBuf>, args: Vec<String>, command_mode: bool, new: N,
    servers: S,
) -> Result<ExitStatus>
where
    R: Runtime,
    N: FnOnce(Compiled<R::StoreCtx>) -> NFut,
    NFut: Future<Output = Result<R>>,
    S: for<'a> FnOnce(&'a R) -> Vec<BoxFuture<'a, Result<()>>>,
{
    let compiled = RegistryBuilder::new()
        .wasm(wasm)
        .config(config)
        .args(args)
        .command(command_mode)
        .compile::<R::StoreCtx>()
        .await
        .context("building runtime")?;

    let runtime = new(compiled).await.context("preparing runtime state")?;

    if command_mode {
        command::run(&runtime).await
    } else {
        prepare(&runtime).await?;
        future::try_join_all(servers(&runtime)).await?;

        Ok(ExitStatus::SUCCESS)
    }
}

// Start a runtime's background tasks and wire host-mediated links — the shared
pub async fn prepare<R: Runtime>(runtime: &R) -> Result<()> {
    // Drive epoch interruption so guest deadlines (and the wall-clock timeouts
    // wrapped around each invocation) fire even while a guest executes
    // CPU-bound code.
    drive_epoch(runtime.registry().engine().clone(), runtime.options().epoch_tick);

    // Periodically sample pool occupancy as metrics so pool sizing can be tuned
    // from real data.
    sample_pool(runtime.registry().engine().clone(), runtime.options().pool_metrics_interval);

    // Wire the serve side of any host-mediated links before triggers fire, so a
    // dispatched call always finds its target's wRPC server.
    serve_links(runtime).await.context("wiring host-mediated link serve side")?;

    Ok(())
}

/// A guest's process exit status.
///
/// # Truncation
///
/// [`code`](Self::code) preserves the full `i32` a guest reports, but a process
/// exit status is only 8 bits on POSIX. The [`ExitCode`](std::process::ExitCode)
/// conversion (and [`code_u8`](Self::code_u8)) therefore keeps just the low
/// byte, matching the `wasmtime` CLI: `256` becomes `0`, `257` becomes `1`, and
/// `-1` becomes `255`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ExitStatus(i32);

impl ExitStatus {
    /// The success status (exit code `0`).
    pub const SUCCESS: Self = Self(0);

    /// The wrapped exit code, as the guest reported it (full `i32`).
    #[must_use]
    pub const fn code(self) -> i32 {
        self.0
    }

    /// The exit code truncated to the low 8 bits — the value a process actually
    /// surfaces on POSIX (and what the [`ExitCode`](std::process::ExitCode)
    /// conversion uses). See [the truncation note](Self#truncation).
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

#[cfg(test)]
mod tests {
    use super::ExitStatus;

    #[test]
    fn success_is_zero() {
        assert_eq!(ExitStatus::SUCCESS.code(), 0);
        assert_eq!(ExitStatus::SUCCESS.code_u8(), 0);
    }

    #[test]
    fn from_i32_preserves_full_code() {
        // `code()` keeps the whole i32; only the byte view / `ExitCode`
        // conversion truncates.
        assert_eq!(ExitStatus::from(2).code(), 2);
        assert_eq!(ExitStatus::from(256).code(), 256);
        assert_eq!(ExitStatus::from(-1).code(), -1);
    }

    #[test]
    fn code_u8_keeps_low_byte() {
        assert_eq!(ExitStatus::from(0).code_u8(), 0);
        assert_eq!(ExitStatus::from(2).code_u8(), 2);
        assert_eq!(ExitStatus::from(255).code_u8(), 255);
        assert_eq!(ExitStatus::from(256).code_u8(), 0);
        assert_eq!(ExitStatus::from(257).code_u8(), 1);
        assert_eq!(ExitStatus::from(-1).code_u8(), 255);
        // The `ExitCode` conversion runs (its value is opaque, so only the
        // byte rule above is asserted).
        let _ = std::process::ExitCode::from(ExitStatus::from(2));
    }

    #[test]
    fn is_copy_and_eq() {
        let status = ExitStatus::from(7);
        let copied = status;
        assert_eq!(status, copied);
    }
}
