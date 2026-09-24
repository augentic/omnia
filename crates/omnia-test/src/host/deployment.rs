//! A manifest-driven command deployment run over a backend bundle.

use anyhow::{Context as _, Result};
use omnia::{
    DeploymentBuilder, ExitStatus, GuestEntry, Host, LevelFilter, Manifest, ManifestSource, Mode,
    Mount, Provides, RegistryConfig, Runtime, Server, SourceSpec, StoreCtx, Wiring,
};
use omnia_wasi_otel::WasiOtel;

/// One command-mode deployment: guests, mounts, arguments, registries, and the tracing level.
///
/// What guests call between themselves is read off their components,
/// declared nowhere; `registries` is the routing an on-demand guest's
/// package source is fetched through.
///
/// Built from nothing, or as an overlay on the manifest a production
/// `runtime!` compiled in (`Deployment::from(runtime::manifest())`): the
/// builder methods add to that base, `command` re-marks its command guest,
/// and a `mount` sharing a base mount's name replaces it (last wins), so a
/// test serves the binary's `.` root from a scratch directory. Drive it
/// through the generated wiring with [`run_with`](Self::run_with), or link
/// hosts by hand with [`run`](Self::run); either way the guest loader is
/// assembly's, serving the deployment's [`on_demand`](Self::on_demand)
/// guests.
///
/// ```no_run
/// use omnia::ExitStatus;
/// use omnia_test::host::{Backends, Deployment};
///
/// # async fn example(requester: &'static str, plugin: &'static str) -> anyhow::Result<()> {
/// let status = Deployment::new()
///     .guest("requester", requester)
///     .on_demand("plugin", plugin)
///     .run(Backends::defaults().await, |_| Ok(()))
///     .await?;
/// assert_eq!(status, ExitStatus::SUCCESS);
/// # Ok(())
/// # }
/// ```
#[derive(Clone, Debug, Default)]
pub struct Deployment {
    base: Option<ManifestSource>,
    guests: Vec<GuestEntry>,
    command: Option<String>,
    mounts: Vec<Mount>,
    args: Vec<String>,
    registries: Option<RegistryConfig>,
    level: Option<LevelFilter>,
}

impl From<ManifestSource> for Deployment {
    fn from(base: ManifestSource) -> Self {
        Self {
            base: Some(base),
            ..Self::default()
        }
    }
}

impl Deployment {
    /// An empty deployment.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Adds a guest under `name` from a component path or embedded bytes.
    #[must_use]
    pub fn guest(self, name: impl Into<String>, source: impl Into<SourceSpec>) -> Self {
        self.entry(GuestEntry::new(name, source))
    }

    /// Declares `name` for on-demand loading from a component path or
    /// embedded bytes: admitted when a guest first `load`s it, not at boot.
    #[must_use]
    pub fn on_demand(self, name: impl Into<String>, source: impl Into<SourceSpec>) -> Self {
        self.entry(GuestEntry::new(name, source).on_demand())
    }

    /// Adds a `[[guest]]` entry as built — a pinned or package source, say.
    #[must_use]
    pub fn entry(mut self, entry: GuestEntry) -> Self {
        self.guests.push(entry);
        self
    }

    /// Marks the guest `name` as the `wasi:cli/run` target (unmarking any
    /// the base manifest marked); without it the sole exporter is the
    /// catch-all.
    #[must_use]
    pub fn command(mut self, name: impl Into<String>) -> Self {
        self.command = Some(name.into());
        self
    }

    /// Preopens `mount` into the guest sandbox, replacing a base mount of
    /// the same name.
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

    /// The wasm-pkg configuration on-demand package sources are fetched
    /// through, replacing the base manifest's.
    #[must_use]
    pub fn registries(mut self, config: impl Into<RegistryConfig>) -> Self {
        self.registries = Some(config.into());
        self
    }

    /// The tracing level for the run: every guest's `RUST_LOG`, whatever the
    /// test process sets, so a suite scripts a guest's level without
    /// touching its own environment.
    ///
    /// Unset, a guest keeps the process `RUST_LOG` and falls back to the
    /// command-mode `info` when it sets none.
    #[must_use]
    pub const fn level(mut self, level: LevelFilter) -> Self {
        self.level = Some(level);
        self
    }

    /// The manifest this overlay describes.
    ///
    /// # Errors
    ///
    /// Returns an error if a `manifest:` base manifest cannot be loaded.
    pub fn manifest(&self) -> Result<Manifest> {
        let base = self.base.clone().map(ManifestSource::into_manifest).transpose()?;
        let mut manifest = base.unwrap_or_default().mounts(self.mounts.iter().cloned());
        if let Some(registries) = &self.registries {
            manifest = manifest.registries(registries.clone());
        }
        for guest in &self.guests {
            manifest = manifest.guest(guest.clone());
        }
        if let Some(command) = &self.command {
            for guest in &mut manifest.guests {
                guest.command = guest.name == *command;
            }
        }
        Ok(manifest)
    }

    fn builder(&self, manifest: Manifest) -> DeploymentBuilder {
        let builder =
            DeploymentBuilder::new().manifest(manifest).mode(Mode::Command).args(self.args.clone());
        match self.level {
            Some(level) => builder.level(level),
            None => builder,
        }
    }

    /// Assembles the runtime by hand: builds the deployment, links the
    /// caller's hosts through `link`, and assembles — which links the guest
    /// loader, installs the on-demand guests as its table, and serves every
    /// guest's linked exports.
    ///
    /// # Errors
    ///
    /// Returns an error if the deployment cannot be built, linked, or
    /// assembled.
    pub async fn boot<B>(
        &self, backends: B, link: impl FnOnce(&mut omnia::Deployment<StoreCtx<B>>) -> Result<()>,
    ) -> Result<Runtime<B>>
    where
        B: Clone + Send + Sync + 'static,
    {
        let mut deployment = self
            .builder(self.manifest()?)
            .build::<StoreCtx<B>>()
            .await
            .context("building deployment")?;
        link(&mut deployment).context("linking hosts")?;
        deployment.assemble(backends).await
    }

    /// Boots by hand, drives the command guest once, and shuts the runtime
    /// down.
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
        let status = runtime.run_command().await;
        runtime.shutdown();
        status
    }

    /// [`Deployment::run`] linking the host under test, `H`, plus the
    /// telemetry host every `omnia_sdk::command!` guest imports.
    ///
    /// A suite testing `WasiOtel` itself, or a bundle without an otel
    /// backend, links by hand through [`run`](Self::run).
    ///
    /// # Errors
    ///
    /// Same as [`Deployment::run`].
    pub async fn run_host<H, B>(&self, backends: B) -> Result<ExitStatus>
    where
        H: Host<StoreCtx<B>> + Server<B>,
        B: Provides<WasiOtel> + Clone + Send + Sync + 'static,
    {
        self.run(backends, |deployment| {
            deployment.host::<H, B>()?;
            deployment.host::<WasiOtel, B>()?;
            Ok(())
        })
        .await
    }

    /// Drives the command guest once through a production `runtime!`'s
    /// wiring (`runtime::Hooks`) over `backends` — the same `link` and
    /// `serve` the binary runs, connecting nothing.
    ///
    /// # Errors
    ///
    /// Returns an error if the deployment cannot be built or assembled, or
    /// the guest traps without exiting.
    pub async fn run_with<H, B>(&self, backends: B) -> Result<ExitStatus>
    where
        H: Wiring<B>,
        B: Clone + Send + Sync + 'static,
    {
        let deployment = self.builder(self.manifest()?).build::<StoreCtx<B>>().await?;
        omnia::run_with::<B, H>(deployment, backends).await
    }
}
