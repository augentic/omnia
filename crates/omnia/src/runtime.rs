//! Deployment lifecycle: [`prepare`], [`run`], background tasks, and [`ExitStatus`].

mod command;

/// Compile-time guard that a `command: true` deployment includes no long-lived
/// trigger server.
///
/// The `runtime!` macro emits
/// `const _: () = omnia::assert_hosts(&[<Host as Server<_>>::IS_SERVER, …]);`
/// for a command deployment, so listing a trigger host (`WasiHttp`,
/// `WasiMessaging`, `WasiWebSocket`) is a build error rather than a silently
/// dropped host. The values come straight from [`Server::IS_SERVER`](crate::Server::IS_SERVER), so a newly
/// added trigger is covered without editing the macro.
///
/// # Panics
///
/// Panics if any element is `true` (a host is a long-lived trigger server). In
/// the macro's const context this surfaces as a compile error.
#[doc(hidden)]
pub const fn assert_hosts(hosts: &[bool]) {
    let mut index = 0;
    while index < hosts.len() {
        assert!(
            !hosts[index],
            "a `command: true` deployment cannot link a long-lived trigger server (`WasiHttp`, \
             `WasiMessaging`, `WasiWebSocket`): a command runs to completion and exits, but the \
             server would run forever. Use the default `command: false` for a server deployment, \
             or drop the trigger host — capability hosts (`WasiKeyValue`, `WasiBlobstore`, ...) \
             are fine to link."
        );
        index += 1;
    }
}

use std::path::PathBuf;
use std::process::ExitCode;
use std::sync::Arc;
use std::time::Duration;

use anyhow::{Context as _, Result};
use clap::Parser as _;
use futures::future::{self, BoxFuture};
use wasmtime::component::{Instance, InstancePre};
use wasmtime::{Engine, Store};

use crate::cli::{Cli, Command};
use crate::dispatch::serve_links;
use crate::traits::{Backends, HasLimits};
use crate::working_tree::WorkingTreeRegistry;
use crate::{Deployment, DeploymentBuilder, Registry, RuntimeOptions, StoreBase, StoreCtx};

/// Host linking and trigger-server startup for a deployment.
pub trait RuntimeHooks<B: Backends> {
    /// Link every declared host into the deployment linker.
    ///
    /// # Errors
    ///
    /// Returns an error if a host cannot be added to the linker.
    fn link(deployment: &mut Deployment<StoreCtx<B>>) -> Result<()>;

    /// Start every long-lived trigger server ([`Server::IS_SERVER`]).
    fn servers(runtime: &Runtime<B>) -> Vec<BoxFuture<'_, Result<()>>>;
}

/// CLI entry point for generated `main` functions.
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

/// Build runtime state, prepare it, then run command mode or every trigger server.
///
/// # Errors
///
/// Returns an error if the deployment cannot be built, runtime state cannot be
/// prepared, link servers cannot be wired, or a trigger server exits with an error.
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

    prepare(&runtime).await?;

    if cmd {
        command::drive(&runtime).await
    } else {
        future::try_join_all(H::servers(&runtime)).await?;
        Ok(ExitStatus::SUCCESS)
    }
}

/// Start background tasks and wire host-mediated link servers.
pub async fn prepare<B>(runtime: &Runtime<B>) -> Result<()>
where
    B: Clone + Send + Sync + 'static,
{
    drive_epoch(runtime.registry().engine().clone(), runtime.options().epoch_tick);
    sample_pool(runtime.registry().engine().clone(), runtime.options().pool_metrics_interval);
    serve_links(runtime).await.context("wiring host-mediated link serve side")?;
    Ok(())
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

/// Connected host runtime: registry, argv, working trees, and backend bundle.
pub struct Runtime<B: 'static> {
    registry: Arc<Registry<StoreCtx<B>>>,
    args: Arc<Vec<String>>,
    working_trees: Arc<WorkingTreeRegistry>,
    backends: B,
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
        let working_trees = deployment.working_trees();

        Ok(Self {
            registry: Arc::new(deployment.build().context("assembling registry")?),
            args,
            working_trees,
            backends,
        })
    }
}

// Manual: `StoreCtx<B>` is not `Clone`; fields here are `Arc`-backed or clone the bundle.
impl<B: Clone + Send + Sync + 'static> Clone for Runtime<B> {
    fn clone(&self) -> Self {
        Self {
            registry: Arc::clone(&self.registry),
            args: Arc::clone(&self.args),
            working_trees: Arc::clone(&self.working_trees),
            backends: self.backends.clone(),
        }
    }
}

impl<B: Clone + Send + Sync + 'static> Runtime<B> {
    /// Build a runtime from an already-assembled registry and backend bundle.
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

    /// Guest registry.
    #[must_use]
    pub fn registry(&self) -> &Registry<StoreCtx<B>> {
        &self.registry
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
            .dispatch(Arc::new(self.clone()))
            .args(&self.args)
            .working_trees(Arc::clone(&self.working_trees))
            .build();
        StoreCtx {
            base,
            backends: self.backends.clone(),
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
    use super::{ExitStatus, assert_hosts};

    #[test]
    fn capability_only_is_allowed() {
        // A command deployment with only capability hosts (every `IS_SERVER`
        // false) is fine.
        assert_hosts(&[false, false, false]);
    }

    #[test]
    fn empty_is_allowed() {
        assert_hosts(&[]);
    }

    #[test]
    #[should_panic(expected = "long-lived trigger server")]
    fn long_lived_server_is_rejected() {
        // Any `true` (a long-lived trigger) in a command deployment fails.
        assert_hosts(&[false, true]);
    }

    #[test]
    fn success_is_zero() {
        assert_eq!(ExitStatus::SUCCESS.code(), 0);
        assert_eq!(ExitStatus::SUCCESS.code_u8(), 0);
    }

    #[test]
    fn from_i32_preserves_full_code() {
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
        let _ = std::process::ExitCode::from(ExitStatus::from(2));
    }

    #[test]
    fn is_copy_and_eq() {
        let status = ExitStatus::from(7);
        let copied = status;
        assert_eq!(status, copied);
    }
}
