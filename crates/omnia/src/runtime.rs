//! # Runtime lifecycle
//!
//! The startup [`prepare`] every deployment shares and the long-lived server
//! loop driven by [`run`], plus the detached background tasks they drive off the
//! Wasmtime [`Engine`] (epoch interruption so guest deadlines fire while
//! CPU-bound guests execute, and pooling-allocator occupancy sampling emitted
//! as `OpenTelemetry` gauges via the `tracing` metrics bridge) and the
//! [`ExitStatus`] a deployment yields.

use std::path::PathBuf;
use std::process::ExitCode;
use std::sync::Arc;
use std::time::Duration;

use anyhow::{Context as _, Result};
use clap::Parser as _;
use futures::future::{self, BoxFuture};
use wasmtime::component::{Instance, InstancePre};
use wasmtime::{Engine, Store};

use crate::dispatch::serve_links;
use crate::traits::{Backends, HasLimits};
use crate::working_tree::WorkingTreeRegistry;
use crate::{
    Cli, Command, Deployment, DeploymentBuilder, Registry, RuntimeOptions, StoreBase, StoreCtx,
    command,
};

/// Per-deployment wiring supplied by the `runtime!` macro (or a hand-written
/// host).
///
/// [`link`](Self::link) runs inside [`Runtime::new`] before backends connect;
/// [`servers`](Self::servers) runs from [`run`] after [`prepare`], starting every
/// long-lived trigger host.
pub trait RuntimeHooks<B: Backends> {
    /// Link every declared host into the deployment linker.
    ///
    /// # Errors
    ///
    /// Returns an error if any host cannot be added to the linker.
    fn link(deployment: &mut Deployment<StoreCtx<B>>) -> Result<()>;

    /// Start every long-lived trigger server ([`Server::IS_SERVER`]).
    fn servers(runtime: &Runtime<B>) -> Vec<BoxFuture<'_, Result<()>>>;
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
pub async fn main<B, H>(cmd: bool) -> ExitCode
where
    B: Backends,
    H: RuntimeHooks<B>,
{
    match Cli::parse().command {
        Command::Run { wasm, config, args } => match run::<B, H>(wasm, config, args, cmd).await {
            Ok(status) => status.into(),
            Err(error) => {
                eprintln!("{error:#}");
                ExitCode::FAILURE
            }
        },
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
pub async fn run<B, H>(
    wasm: Option<PathBuf>, config: Option<PathBuf>, args: Vec<String>, cmd: bool,
) -> Result<ExitStatus>
where
    B: Backends,
    H: RuntimeHooks<B>,
{
    let deployment = DeploymentBuilder::new()
        .wasm(wasm)
        .config(config)
        .args(args)
        .command(cmd)
        .build::<StoreCtx<B>>()
        .await
        .context("building runtime")?;

    let runtime =
        Runtime::<B>::new(deployment, H::link).await.context("preparing runtime state")?;

    if cmd {
        command::run(&runtime).await
    } else {
        prepare(&runtime).await?;
        future::try_join_all(H::servers(&runtime)).await?;
        Ok(ExitStatus::SUCCESS)
    }
}

// Start a runtime's background tasks and wire host-mediated links — the shared
pub async fn prepare<B>(runtime: &Runtime<B>) -> Result<()>
where
    B: Clone + Send + Sync + 'static,
{
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

// Spawn a detached background task that drives epoch interruption.
fn drive_epoch(engine: Engine, tick: Duration) {
    tokio::spawn(async move {
        let mut interval = tokio::time::interval(tick);
        loop {
            interval.tick().await;
            engine.increment_epoch();
        }
    });
}

// Spawn a detached background task that samples pool occupancy as metrics.
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

/// The host runtime the `runtime!` macro builds for a deployment.
///
/// It owns the fixed runtime state every deployment shares — the guest
/// [`Registry`], the guest argv, and the working-tree registry — plus the
/// deployment's connected [`Backends`] bundle `B`. The per-store context is the
/// library's [`StoreCtx<B>`](crate::StoreCtx): its fixed views live in `omnia`,
/// and each host crate blankets its own view over `StoreCtx<B>`, so the
/// deployment supplies only the bundle and its `HasXxx` accessor impls.
///
/// The macro previously emitted this struct, its [`new`](Self::new), and a
/// runtime trait impl inline; hosting it here keeps that boilerplate (and the
/// backend-connection lifecycle) in the library.
pub struct Runtime<B: 'static> {
    registry: Arc<Registry<StoreCtx<B>>>,
    args: Arc<Vec<String>>,
    working_trees: Arc<WorkingTreeRegistry>,
    backends: B,
}

// A derived `Clone` would demand `StoreCtx<B>: Clone`, but it is never `Clone`
// (it owns the WASI table); every field here is either shared behind an `Arc` or
// comes from the `Clone` bundle, so only `B: Clone` is required.
impl<B: Clone + 'static> Clone for Runtime<B> {
    fn clone(&self) -> Self {
        Self {
            registry: Arc::clone(&self.registry),
            args: Arc::clone(&self.args),
            working_trees: Arc::clone(&self.working_trees),
            backends: self.backends.clone(),
        }
    }
}

impl<B: Backends> Runtime<B> {
    /// Assemble the runtime from a [`Deployment`]: thread the guest argv, link
    /// the deployment's hosts (via `link`), connect every backend, and freeze
    /// the guest [`Registry`].
    ///
    /// `link` is the one per-deployment step the macro still supplies — typically
    /// [`RuntimeHooks::link`], which calls [`Deployment::host`] once per wired
    /// host ([`RuntimeHooks::servers`] mirrors it when starting trigger servers).
    ///
    /// # Errors
    ///
    /// Returns an error if host linking, backend connection, or registry
    /// assembly fails.
    pub async fn new<L>(mut deployment: Deployment<StoreCtx<B>>, link: L) -> Result<Self>
    where
        L: FnOnce(&mut Deployment<StoreCtx<B>>) -> Result<()>,
    {
        let args = Arc::new(deployment.args().to_vec());
        link(&mut deployment).context("linking hosts")?;
        let backends = B::connect().await.context("connecting backends")?;
        let working_trees = deployment.working_trees();

        Ok(Self {
            registry: Arc::new(deployment.build().context("assembling registry")?),
            args,
            working_trees,
            backends,
        })
    }
}

impl<B: Clone + Send + Sync + 'static> Runtime<B> {
    /// Assemble the runtime from already-built parts.
    ///
    /// The lower-level counterpart to [`new`](Self::new): it takes a frozen
    /// [`Registry`] and an already-connected backend bundle directly, bypassing
    /// host linking and [`Backends::connect`]. The `runtime!` macro path uses
    /// [`new`](Self::new); this serves tests and hand-written hosts that assemble
    /// the registry and bundle themselves.
    #[must_use]
    pub fn from_parts(
        registry: Arc<Registry<StoreCtx<B>>>, args: Vec<String>,
        working_trees: Arc<WorkingTreeRegistry>, backends: B,
    ) -> Self {
        Self {
            registry,
            args: Arc::new(args),
            working_trees,
            backends,
        }
    }

    /// Returns the multi-guest registry.
    #[must_use]
    pub fn registry(&self) -> &Registry<StoreCtx<B>> {
        &self.registry
    }

    /// Returns the environment-derived runtime options (the registry's options).
    #[must_use]
    pub fn options(&self) -> &RuntimeOptions {
        self.registry().options()
    }

    /// Build a fresh per-guest store context.
    ///
    /// `HostDispatch` is implemented for [`Runtime<B>`], so a fresh clone backs
    /// any host->guest call lodged in the store.
    #[must_use]
    pub fn store(&self) -> StoreCtx<B> {
        let base = StoreBase::builder()
            .options(self.options())
            .dispatch(Arc::new(self.clone()))
            .args(&self.args)
            .working_trees(Arc::clone(&self.working_trees))
            .build();
        StoreCtx {
            base,
            backends: self.backends.clone(),
        }
    }

    /// Build a fully configured [`Store`] for a single guest invocation.
    ///
    /// Installs an epoch deadline (so CPU-bound guests periodically yield to the
    /// async executor, allowing an enclosing wall-clock timeout to fire), an
    /// optional fuel budget, and the per-guest memory limiter.
    #[must_use]
    pub fn build_store(&self, data: StoreCtx<B>) -> Store<StoreCtx<B>> {
        let options = self.options();
        let mut store = Store::new(self.registry().engine(), data);

        // Yield to the executor every epoch tick; the deadline is bumped on each
        // yield so execution continues until a surrounding `timeout` cancels it.
        store.set_epoch_deadline(1);
        store.epoch_deadline_async_yield_and_update(1);

        if options.max_fuel > 0 {
            // `consume_fuel` is enabled in `compile_config` whenever a budget is
            // set, so this only fails on a compile/run configuration mismatch.
            if let Err(error) = store.set_fuel(options.max_fuel) {
                tracing::warn!(%error, "failed to set fuel budget");
            }
        }

        store.limiter(|ctx| ctx.limits());
        store
    }

    /// Instantiate a selected guest's pre-instantiated component into `store`,
    /// recording instantiation latency (the `instantiation_duration_us`
    /// histogram) and failures (the `pool_instantiation_errors` counter, a proxy
    /// for pool exhaustion) as `OpenTelemetry` metrics.
    ///
    /// The caller passes the [`InstancePre`] resolved from the registry (the
    /// default guest, or an identity-selected one) so a dispatched call lands in
    /// a fresh instance.
    ///
    /// # Errors
    ///
    /// Returns an error if the component cannot be instantiated, e.g. when the
    /// pooling allocator is exhausted.
    pub async fn instantiate(
        &self, instance_pre: &InstancePre<StoreCtx<B>>, store: &mut Store<StoreCtx<B>>,
    ) -> Result<Instance> {
        match instance_pre.instantiate_async(store).await {
            Ok(instance) => {
                tracing::debug!("component instantiated");
                Ok(instance)
            }
            Err(error) => Err(error.into()),
        }
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
