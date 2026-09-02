//! A manifest-driven command deployment run over a backend bundle.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use anyhow::{Context as _, Result};
use omnia::wasmtime_wasi::I32Exit;
use omnia::wasmtime_wasi::p3::bindings::Command;
use omnia::{
    DeploymentBuilder, ExitStatus, GuestEntry, GuestId, Host, Manifest, Mode, Mount, PathMounts,
    Plugins, Runtime, Server, SourceSpec, StoreCtx, WasiPlugins, as_command_chain, serve_links,
};

/// One command-mode deployment: guests, mounts, arguments, the plugin seams
/// the host mediates, and the directory the path-load slot serves.
///
/// `run` assembles what a production `runtime!` declares — build, link,
/// registry, runtime, plugin loader, link serve side — drives `wasi:cli/run`
/// once, and shuts the runtime down.
///
/// ```no_run
/// use omnia::ExitStatus;
/// use omnia_test::host::{Backends, Deployment, scratch};
///
/// # async fn example(requester: &'static str, plugin: &'static str) -> anyhow::Result<()> {
/// let scratch = scratch();
/// std::fs::copy(plugin, scratch.path().join("plugin.wasm"))?;
/// let status = Deployment::new()
///     .plugins(["acme:tools/ops"])
///     .guest("requester", requester)
///     .mount(scratch.mount(false))
///     .path_root(scratch.path())
///     .run(Backends::defaults().await, |_| Ok(()))
///     .await?;
/// assert_eq!(status, ExitStatus::SUCCESS);
/// # Ok(())
/// # }
/// ```
#[derive(Clone, Debug, Default)]
pub struct Deployment {
    guests: Vec<GuestEntry>,
    command: Option<String>,
    mounts: Vec<Mount>,
    args: Vec<String>,
    plugins: Vec<String>,
    path_root: Option<PathBuf>,
}

impl Deployment {
    /// An empty deployment.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Adds a guest under `id` from a component path or embedded bytes.
    #[must_use]
    pub fn guest(mut self, id: impl Into<String>, source: impl Into<SourceSpec>) -> Self {
        self.guests.push(GuestEntry::new(id, source));
        self
    }

    /// Marks `id` as the `wasi:cli/run` target; without it the sole exporter
    /// is the catch-all.
    #[must_use]
    pub fn command(mut self, id: impl Into<String>) -> Self {
        self.command = Some(id.into());
        self
    }

    /// Preopens `mount` into the guest sandbox.
    #[must_use]
    pub fn mount(mut self, mount: Mount) -> Self {
        self.mounts.push(mount);
        self
    }

    /// Preopens every mount into the guest sandbox.
    #[must_use]
    pub fn mounts(mut self, mounts: impl IntoIterator<Item = Mount>) -> Self {
        self.mounts.extend(mounts);
        self
    }

    /// The operator's arguments (the runtime supplies `argv[0]`).
    #[must_use]
    pub fn args<S: Into<String>>(mut self, args: impl IntoIterator<Item = S>) -> Self {
        self.args.extend(args.into_iter().map(Into::into));
        self
    }

    /// Interfaces the host mediates between guests; the `omnia:plugins`
    /// loader host is linked whenever any are declared.
    #[must_use]
    pub fn plugins<S: Into<String>>(mut self, interfaces: impl IntoIterator<Item = S>) -> Self {
        self.plugins.extend(interfaces.into_iter().map(Into::into));
        self
    }

    /// Serves path loads (`Location::Path`) from `dir`, mounted as `.`.
    #[must_use]
    pub fn path_root(mut self, dir: impl AsRef<Path>) -> Self {
        self.path_root = Some(dir.as_ref().to_path_buf());
        self
    }

    fn manifest(&self) -> Manifest {
        let mut manifest = Manifest::new().mounts(self.mounts.iter().cloned());
        for guest in &self.guests {
            let mut guest = guest.clone();
            guest.command = self.command.as_deref() == Some(guest.id.as_str());
            manifest = manifest.guest(guest);
        }
        manifest.plugins(self.plugins.iter().cloned())
    }

    /// Assembles the runtime: builds the deployment, links the plugin host
    /// when seams are declared and the caller's hosts through `link`, then
    /// installs the path loader and wires the link serve side.
    ///
    /// # Errors
    ///
    /// Returns an error if the deployment cannot be built or linked, the path
    /// root cannot be opened, or the link serve side cannot be wired.
    pub async fn boot<B>(
        &self, backends: B, link: impl FnOnce(&mut omnia::Deployment<StoreCtx<B>>) -> Result<()>,
    ) -> Result<Runtime<B>>
    where
        B: Clone + Send + Sync + 'static,
    {
        let mut built = DeploymentBuilder::new()
            .manifest(self.manifest())
            .mode(Mode::Command)
            .args(self.args.clone())
            .build::<StoreCtx<B>>()
            .await
            .context("building deployment")?;
        if !self.plugins.is_empty() || self.path_root.is_some() {
            built.host::<WasiPlugins, B>().context("linking the plugins host")?;
        }
        link(&mut built).context("linking hosts")?;

        let mounts = built.mounts();
        let args = built.args().to_vec();
        let registry = Arc::new(built.into_registry().context("assembling registry")?);
        let runtime = Runtime::from_parts(registry, args, mounts, backends);

        if let Some(root) = &self.path_root {
            let path = PathMounts::new([(".", root.as_path())])
                .with_context(|| format!("opening {} as the path-load root", root.display()))?;
            Plugins::install(&runtime, None, Some(Arc::new(path)))
                .context("installing the plugin loader")?;
        }
        serve_links(&runtime).await.context("wiring host-mediated dispatch")?;
        Ok(runtime)
    }

    /// Boots, drives the command guest once, and shuts the runtime down.
    ///
    /// # Errors
    ///
    /// Same as [`Deployment::boot`], or if the guest traps without exiting.
    pub async fn run<B>(
        &self, backends: B, link: impl FnOnce(&mut omnia::Deployment<StoreCtx<B>>) -> Result<()>,
    ) -> Result<ExitStatus>
    where
        B: Clone + Send + Sync + 'static,
    {
        let runtime = self.boot(backends, link).await?;
        let status = match &self.command {
            Some(id) => drive(&runtime, &GuestId::from(id.as_str())).await,
            None => runtime.run_command().await,
        };
        runtime.shutdown();
        status
    }

    /// [`Deployment::run`] linking exactly one host.
    ///
    /// # Errors
    ///
    /// Same as [`Deployment::run`].
    pub async fn run_host<H, B>(&self, backends: B) -> Result<ExitStatus>
    where
        H: Host<StoreCtx<B>> + Server<B>,
        B: Clone + Send + Sync + 'static,
    {
        self.run(backends, |deployment| {
            deployment.host::<H, B>()?;
            Ok(())
        })
        .await
    }
}

// The runtime's own command drive resolves an explicitly marked guest only
// through `Runtime::new`, which connects backends from the environment; a
// bundle built by hand needs the same drive against a named guest.
async fn drive<B>(runtime: &Runtime<B>, id: &GuestId) -> Result<ExitStatus>
where
    B: Clone + Send + Sync + 'static,
{
    let guest = runtime
        .registry()
        .get(id)
        .with_context(|| format!("command guest `{id}` is not registered"))?;
    let mut store = runtime.build_store(runtime.store());
    let instance = runtime.instantiate(guest.instance_pre(), &mut store).await?;
    let command = Command::new(&mut store, &instance)?;
    let outcome = as_command_chain(
        store.run_concurrent(async move |store| command.wasi_cli_run().call_run(store).await),
    )
    .await;
    match outcome {
        Ok(Ok(Ok(()))) => Ok(ExitStatus::SUCCESS),
        Ok(Ok(Err(()))) => Ok(ExitStatus::from(1)),
        Ok(Err(error)) | Err(error) => {
            let exit = error.downcast_ref::<I32Exit>().map(|exit| exit.0);
            exit.map_or_else(|| Err(error.into()), |code| Ok(ExitStatus::from(code)))
        }
    }
}
