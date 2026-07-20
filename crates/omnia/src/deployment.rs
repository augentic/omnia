//! # WebAssembly Initiator

mod manifest;
mod source;

use std::collections::BTreeSet;
use std::env;
use std::sync::Arc;

use anyhow::{Context, Result};
pub use manifest::{
    GuestEntry, HttpRoute, Manifest, Mount, RouteSpec, SourceSpec, TopicRoute, Transport,
    TransportKind,
};
pub use source::{LoadedGuest, Source};
use wasmtime::component::Linker;
use wasmtime::{Config, Engine};
use wasmtime_wasi::WasiView;
use wrpc_wasmtime::WrpcView;

use crate::dispatch::{DispatchHandle, FirstArgSelector, GuestSelector};
use crate::mount::{MountRegistry, ResolvedPreopen};
use crate::registry::{Registry, Routes};
use crate::{Host, Mode, RuntimeOptions, Server, Telemetry};

/// Builds a [`Deployment`] from an optional programmatic [`Manifest`].
///
/// When no manifest is set, [`build`](Self::build) loads the path in
/// `OMNIA_CONFIG`.
///
/// ```ignore
/// let deployment = DeploymentBuilder::new()
///     .manifest(Manifest::from_wasm(wasm))
///     .args(args)
///     .mode(mode)
///     .build::<StoreCtx>()
///     .await?;
/// ```
#[derive(Debug, Default)]
pub struct DeploymentBuilder {
    manifest: Option<Manifest>,
    args: Vec<String>,
    mode: Mode,
}

impl DeploymentBuilder {
    /// Start a new builder with no source selected.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

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

    /// Resolve the manifest into a [`Deployment`].
    ///
    /// If no manifest was supplied, the path in `OMNIA_CONFIG` is loaded.
    ///
    /// # Errors
    ///
    /// Returns an error if no manifest resolves, the manifest is invalid, or
    /// the deployment cannot be built.
    pub async fn build<T: WasiView + 'static>(self) -> Result<Deployment<T>> {
        let manifest = if let Some(manifest) = self.manifest {
            manifest
        } else {
            let config = env::var_os("OMNIA_CONFIG")
                .context("no deployment manifest supplied and OMNIA_CONFIG is unset")?;
            Manifest::from_config(config)?
        };
        manifest.validate()?;

        let plan = Plan {
            name: manifest.name().to_owned(),
            sources: manifest.sources()?,
            routes: manifest.routes(),
            links: manifest.link_interfaces(),
            preopens: manifest.preopens(),
            args: self.args,
            mode: self.mode,
        };

        init_env(&plan.name)?;
        tracing::info!("initializing runtime");

        Deployment::from_plan(plan).await
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
        let loaded =
            futures::future::try_join_all(plan.sources.iter().map(|source| source.load(&engine)))
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
