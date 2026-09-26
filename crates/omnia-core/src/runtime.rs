//! Connected runtime: [`Runtime`], [`RuntimeParts`], [`WeakRuntime`], and [`ExitStatus`].

mod command;

use std::fmt;
use std::sync::{Arc, Weak};

use anyhow::Result;
use wasmtime::Store;
use wasmtime::component::{Instance, InstancePre};

use crate::artifact::component;
use crate::extensions::Extensions;
use crate::mount::MountRegistry;
use crate::registry::{Guest, GuestId, HttpRoutes, PublishError, TriggerRouter};
use crate::source::{AcquireError, RegistrySource, SourceSpec, Verified};
use crate::store::HasLimits;
use crate::{ChainCtx, Dispatcher, Registry, RuntimeOptions, StoreBase, StoreCtx};

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

/// Inputs to [`Runtime::from_parts`].
pub struct RuntimeParts<B: 'static> {
    /// Deployment name read by trigger servers and the bootstrap log.
    pub name: Arc<str>,
    /// Assembled guest registry.
    pub registry: Arc<Registry<StoreCtx<B>>>,
    /// Guest argv.
    pub args: Vec<String>,
    /// Mount registry opened from the deployment's preopens — the WASI
    /// preopens of every store.
    pub mounts: Arc<MountRegistry>,
    /// Connected backend bundle.
    pub backends: B,
    /// How a declared package source is fetched at first use. `None` when the
    /// runtime has no registry client (omnia built without the `loader`
    /// feature), where every package guest's first use fails.
    pub packages: Option<Arc<dyn RegistrySource>>,
    /// The deployment's wasm-pkg client configuration (TOML), resolved to its
    /// contents: how a package source is routed to a registry. `None` when
    /// the deployment declares none, where every package source is refused.
    pub registry_config: Option<String>,
    /// The run's tracing directives — its verbosity flag composed with the
    /// process `RUST_LOG` ([`telemetry::directives`](crate::telemetry::directives))
    /// — set as every guest's `RUST_LOG`.
    pub rust_log: String,
    /// Command-mode guest identity, if any.
    pub command_guest: Option<GuestId>,
}

/// Connected host runtime: registry, argv, mounts, and backend bundle.
///
/// A thin handle over shared state: `clone()` bumps two reference counts, so
/// the per-request and per-message handler clones never copy the backend
/// bundle.
pub struct Runtime<B: 'static> {
    inner: Arc<RuntimeInner<B>>,
    // built once so `store()` hands out clones instead of one per store
    dispatcher: Arc<dyn Dispatcher>,
}

struct RuntimeInner<B: 'static> {
    // carried state, never a process environment variable
    name: Arc<str>,
    registry: Arc<Registry<StoreCtx<B>>>,
    args: Arc<Vec<String>>,
    mounts: Arc<MountRegistry>,
    backends: B,
    packages: Option<Arc<dyn RegistrySource>>,
    // absent, command mode routes to the sole static `wasi:cli/run` exporter
    command_guest: Option<GuestId>,
    // for an embedder that installs the loader capability by hand
    registry_config: Option<String>,
    // every store's `RUST_LOG`
    rust_log: String,
    extensions: Extensions,
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
    /// The bytes are not a loadable component, or failed pre-instantiation
    /// against the deployment's host set.
    ArtifactRefused(String),
    /// The identity is already registered — an earlier or racing
    /// registration holds it.
    AlreadyRegistered(String),
    /// Serve wiring or publication failed.
    Internal(String),
}

impl fmt::Display for AdmitError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::ArtifactRefused(reason)
            | Self::AlreadyRegistered(reason)
            | Self::Internal(reason) => f.write_str(reason),
        }
    }
}

impl std::error::Error for AdmitError {}

/// Why [`Runtime::guest`] could not produce the guest an identity names.
#[derive(Clone, Debug)]
pub enum GuestError {
    /// The deployment neither registered nor declares the identity.
    Unregistered(GuestId),
    /// The declared source could not produce its bytes; a retry may succeed.
    Unavailable(String),
    /// The bytes were refused: they miss the pin, are a pre-compiled
    /// artifact where raw wasm alone is admitted, or do not load as a
    /// component against the deployment's host set.
    Refused(String),
    /// The runtime has no registry to fetch a package from, or the
    /// registration itself failed.
    Internal(String),
}

impl fmt::Display for GuestError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Unregistered(id) => write!(f, "guest `{id}` is not registered"),
            Self::Unavailable(reason) | Self::Refused(reason) | Self::Internal(reason) => {
                f.write_str(reason)
            }
        }
    }
}

impl std::error::Error for GuestError {}

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

    /// Build a runtime from already-assembled parts.
    ///
    /// Does not wire the host-mediated link serve side — a caller whose
    /// deployment declares link interfaces must run
    /// [`serve_links`](Self::serve_links) itself before dispatching.
    #[must_use]
    pub fn from_parts(parts: RuntimeParts<B>) -> Self {
        Self::with_inner(Arc::new(RuntimeInner {
            name: parts.name,
            registry: parts.registry,
            args: Arc::new(parts.args),
            mounts: parts.mounts,
            backends: parts.backends,
            packages: parts.packages,
            command_guest: parts.command_guest,
            registry_config: parts.registry_config,
            rust_log: parts.rust_log,
            extensions: Extensions::new(),
        }))
    }

    /// The mounts preopened into every store.
    #[must_use]
    pub fn mounts(&self) -> &MountRegistry {
        &self.inner.mounts
    }

    /// The deployment's wasm-pkg client configuration (TOML), resolved to its
    /// contents, for the guest loader to route package sources through;
    /// `None` when the deployment declares none.
    #[must_use]
    pub fn registry_config(&self) -> Option<&str> {
        self.inner.registry_config.as_deref()
    }

    /// The guest `id` names, loaded from its declared source on first use.
    ///
    /// A registered guest is returned as it stands. One the deployment
    /// declares but has not loaded is read — or fetched, for a package —
    /// verified against its entry, admitted as a late guest, and returned;
    /// when a racing first use admitted it first, that registration stands
    /// and is returned. Every way into a guest resolves through here, so a
    /// declared guest loads the first time a route, the command drive, a
    /// link call, a host dispatch, or the guest loader names it.
    ///
    /// # Errors
    ///
    /// Returns [`GuestError::Unregistered`] for an identity the deployment
    /// neither registered nor declares; `Unavailable` when the source could
    /// not produce its bytes; `Refused` when the bytes miss the pin, are a
    /// pre-compiled artifact where raw wasm alone is admitted, or do not
    /// load as a component; `Internal` when a package has no registry to be
    /// fetched from or the registration failed.
    pub async fn guest(&self, id: &GuestId) -> Result<Arc<Guest<StoreCtx<B>>>, GuestError> {
        let registry = self.registry();
        if let Some(guest) = registry.get(id) {
            return Ok(guest);
        }
        let Some(source) = registry.declared(id) else {
            return Err(GuestError::Unregistered(id.clone()));
        };

        let bytes = match source.spec() {
            SourceSpec::Package(package) => {
                let Some(packages) = &self.inner.packages else {
                    return Err(GuestError::Internal(format!(
                        "guest `{id}` is the package `{package}`, but this runtime has no \
                         registry to fetch it from: build omnia with the `loader` feature"
                    )));
                };
                packages.acquire(package, None).await.map_err(|error| match error {
                    AcquireError::Refused(reason) => GuestError::Refused(reason),
                    AcquireError::Unavailable(reason) => GuestError::Unavailable(reason),
                })?
            }
            SourceSpec::Path(_) | SourceSpec::Bytes(_) => source
                .read()
                .await
                .map_err(|error| GuestError::Unavailable(format!("{error:#}")))?,
        };
        let verified =
            source.verified(bytes).map_err(|error| GuestError::Refused(format!("{error:#}")))?;
        match self.admit(id.clone(), verified).await {
            // a racing first use admitted it first; that registration stands
            Ok(()) | Err(AdmitError::AlreadyRegistered(_)) => {}
            Err(AdmitError::ArtifactRefused(reason)) => return Err(GuestError::Refused(reason)),
            Err(AdmitError::Internal(reason)) => return Err(GuestError::Internal(reason)),
        }
        registry.get(id).ok_or_else(|| {
            GuestError::Internal(format!(
                "guest `{id}` was admitted and deregistered before it could be returned"
            ))
        })
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
    /// `probe` resolves a guest's typed handler indices; a loaded guest is
    /// capable exactly when it succeeds. A route may also name a declared
    /// guest, probed when its first request loads it.
    ///
    /// # Errors
    ///
    /// Returns an error if a route names a guest that neither exports the
    /// handler nor is declared, or two or more guests export it with no
    /// routes.
    pub fn http_trigger_router<I, E, F>(&self, probe: F) -> Result<TriggerRouter<HttpRoutes>>
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
        self.inner.command_guest.as_ref()
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

    /// The capability-crate state installed at assembly — the same set every
    /// store context carries.
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

    /// Fresh per-guest store context at the root of a server chain
    /// ([`ChainCtx::server`]): what a trigger builds for the guest it serves.
    #[must_use]
    pub fn store(&self) -> StoreCtx<B> {
        self.store_in(ChainCtx::server())
    }

    /// Fresh per-guest store context at `chain`: a root for the command
    /// driver, the context a link dispatch derived for its callee.
    ///
    /// The guest's environment is the host's with `RUST_LOG` set to the
    /// deployment's tracing directives: the run's verbosity flag composed
    /// with the process variable, decided once at build.
    #[must_use]
    pub fn store_in(&self, chain: ChainCtx) -> StoreCtx<B> {
        // the environment as it stands; a non-UTF-8 pair cannot cross `wasi:cli`
        let host = std::env::vars_os().filter_map(|(name, value)| {
            Some((name.into_string().ok()?, value.into_string().ok()?))
        });
        let env = crate::store::guest_env(host, &self.inner.rust_log);
        StoreCtx {
            base: StoreBase::new(crate::StoreConfig {
                options: self.options(),
                dispatcher: Arc::clone(&self.dispatcher),
                chain,
                args: Some(Arc::clone(&self.inner.args)),
                mounts: Some(Arc::clone(&self.inner.mounts)),
                env: Some(Arc::new(env)),
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
            // a failure means the engine was built from different options
            store.set_fuel(options.max_fuel).expect("engine was built without fuel metering");
        }

        store.limiter(|ctx| ctx.limits());
        store
    }

    /// A shareable factory for fresh, fully configured guest stores at a given
    /// chain context — what the link serve side hands to each served function
    /// to instantiate the target per call.
    #[must_use]
    pub fn store_factory(&self) -> Arc<dyn Fn(ChainCtx) -> Store<StoreCtx<B>> + Send + Sync> {
        let runtime = self.clone();
        Arc::new(move |chain| runtime.build_store(runtime.store_in(chain)))
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

    /// Register a raw wasm component at run time under `id`. Pre-compiled
    /// bytes are refused here: an embedder holding `omnia compile` output its
    /// own pipeline produced admits it through [`admit`](Self::admit) with an
    /// `unsafe` [`Verified::trusted`].
    ///
    /// The identity is opaque and must not already be registered; an upgrade
    /// is [`deregister`](Self::deregister) + `register` (or a new id). A
    /// failed registration leaves no partial state. The registry entry
    /// records the content digest of `bytes`, as it does for every guest.
    ///
    /// # Errors
    ///
    /// Returns an error if the bytes are a pre-compiled artifact, `id` is
    /// already registered, the bytes cannot be loaded, the component's
    /// imports exceed the deployment's linked host set and declared link
    /// interfaces, or its linked exports cannot be served.
    pub async fn register(&self, id: impl Into<GuestId>, bytes: Vec<u8>) -> Result<()> {
        self.admit(id.into(), Verified::wasm(bytes)?).await.map_err(anyhow::Error::from)
    }

    /// Admit verified component bytes as a late guest under `id`: load them
    /// as a boot guest's are loaded, pre-instantiate against the shared host
    /// set, wire the host-mediated link serve side, then publish entry and
    /// endpoint as one atomic lifecycle transition — no dispatch can ever
    /// resolve the entry and miss the endpoint, or vice versa.
    ///
    /// The token's digest is recorded on the registry entry, so the
    /// attestation lives exactly as long as the entry —
    /// [`Guest::digest`](crate::Guest::digest) reads it back. Acquisition and
    /// digest policy live with the callers: [`guest`](Self::guest) for a
    /// declared source, the guest loader (`omnia-plugin`) for a location a
    /// guest names. Whether the component exports a linked interface is not
    /// checked here: a guest that exports none is still reachable through
    /// the host [`Dispatcher`], and a link call to it fails at the call site.
    ///
    /// # Errors
    ///
    /// Returns a typed [`AdmitError`] naming the refusal: refused artifact,
    /// an identity already registered (an earlier or racing registration), or
    /// an internal serve/publication failure.
    pub async fn admit(&self, id: GuestId, verified: Verified) -> Result<(), AdmitError> {
        let registry = self.registry();

        // early occupancy check; the publish below re-checks transactionally
        if registry.get(&id).is_some() {
            return Err(AdmitError::AlreadyRegistered(format!(
                "guest `{id}` is already registered"
            )));
        }

        let digest = verified.digest();
        let component = component(registry.engine(), verified).await.map_err(|error| {
            AdmitError::ArtifactRefused(format!("validating `{id}`: {error:#}"))
        })?;
        let instance_pre = registry.instantiate_late(&id, &component).map_err(|error| {
            AdmitError::ArtifactRefused(format!("pre-instantiating `{id}`: {error:#}"))
        })?;
        let guest = Guest::local(id.clone(), instance_pre, digest);

        // serve as a pending endpoint, then publish endpoint and entry as one step
        registry.seam().serve(self.store_factory(), &guest).await.map_err(|error| {
            AdmitError::Internal(format!("serving `{id}` seam exports: {error:#}"))
        })?;
        registry.publish(guest).map_err(|error| match error {
            PublishError::Occupied(id) => {
                AdmitError::AlreadyRegistered(format!("guest `{id}` is already registered"))
            }
            PublishError::Transport(error) => {
                AdmitError::Internal(format!("publishing `{id}`: {error:#}"))
            }
        })?;

        tracing::debug!(guest = %id, "guest registered");
        Ok(())
    }

    /// Remove a late guest — one registered at run time, or a declared guest
    /// loaded at first use, which its next use reloads from its source.
    /// In-flight calls complete on the instance they hold
    /// (instance-per-call). Guests loaded at boot are refused.
    ///
    /// # Errors
    ///
    /// Returns an error if `id` names a guest loaded at boot or is not
    /// registered.
    pub fn deregister(&self, id: &GuestId) -> Result<()> {
        self.registry().remove(id)?;
        tracing::debug!(guest = %id, "guest deregistered");
        Ok(())
    }

    /// Release every link-serve endpoint, aborting the drain tasks that pin
    /// `Runtime` clones (and with them the engine's pooling reservation).
    ///
    /// `run` does this as the drive completes; an embedder holding a
    /// [`from_parts`](Self::from_parts) runtime calls it when the deployment
    /// is finished. In-flight invocations hold their own server handles and
    /// complete; only new dispatches are cut off.
    pub fn shutdown(&self) {
        self.registry().seam().shutdown();
    }

    /// Wire the serve side of every registered guest's linked exports, then
    /// publish them all under one lifecycle transition so polyfilled imports
    /// can reach them. `Deployment::assemble` calls this during bootstrap; only
    /// a runtime assembled through [`from_parts`](Self::from_parts) wires it
    /// explicitly. A no-op for a deployment that declares no link interfaces.
    ///
    /// # Errors
    ///
    /// Returns an error if a guest's export cannot be served, or a served guest
    /// already has an endpoint (`serve_links` ran twice).
    pub async fn serve_links(&self) -> Result<()> {
        let registry = self.registry();
        let seam = registry.seam();
        let factory = self.store_factory();
        let guests: Vec<_> = registry.guests().collect();

        // on failure, release what is still parked so a failed bootstrap pins nothing
        let discard_from = |first_unpublished: usize| {
            for guest in &guests[first_unpublished..] {
                seam.discard(guest.id());
            }
        };

        for guest in &guests {
            if let Err(error) = seam.serve(Arc::clone(&factory), guest).await {
                discard_from(0);
                return Err(error);
            }
        }

        let _lifecycle = registry.lifecycle_write();
        for (published, guest) in guests.iter().enumerate() {
            if let Err(error) = seam.publish(guest.id()) {
                discard_from(published + 1);
                return Err(error);
            }
        }
        Ok(())
    }
}

/// Wire the link serve side of every registered guest; see
/// [`Runtime::serve_links`].
///
/// # Errors
///
/// Returns an error if a guest's export cannot be served, or a served guest
/// already has an endpoint.
pub async fn serve_links<B>(runtime: &Runtime<B>) -> Result<()>
where
    B: Clone + Send + Sync + 'static,
{
    runtime.serve_links().await
}

#[cfg(test)]
mod tests {
    use super::ExitStatus;

    #[test]
    fn code_u8_low_byte() {
        // posix low-byte truncation
        assert_eq!(ExitStatus::from(256).code_u8(), 0);
        assert_eq!(ExitStatus::from(257).code_u8(), 1);
        assert_eq!(ExitStatus::from(-1).code_u8(), 255);
    }
}
