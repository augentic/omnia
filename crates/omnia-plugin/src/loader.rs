//! The load path of the `omnia:plugins/loader` capability: pin policy,
//! idempotency, acquisition routing, and admission into the runtime.

use std::collections::BTreeMap;
use std::fmt;
use std::future::Future;
use std::sync::{Arc, Mutex, PoisonError};

use futures::FutureExt as _;
use futures::future::BoxFuture;
use omnia_core::{AdmitError, GuestId, Runtime, WeakRuntime};

use crate::{Acquirer, Location, digest};

/// A loaded plugin handle: the routed identity plus the resolved content
/// digest.
#[derive(Clone, Debug)]
pub struct Plugin {
    id: GuestId,
    digest: Arc<str>,
}

impl Plugin {
    /// The routed identity host-mediated dispatch keys on.
    #[must_use]
    pub const fn id(&self) -> &GuestId {
        &self.id
    }

    /// The resolved `sha256:<hex>` digest of the loaded bytes.
    #[must_use]
    pub fn digest(&self) -> &str {
        &self.digest
    }
}

/// Why a `loader.load` request was refused; each variant carries the
/// refusal's description and maps onto one `omnia:plugins/loader.error`
/// discriminant.
#[derive(Clone, Debug)]
pub enum LoadError {
    /// The deployment's acquirer does not serve the requested location kind.
    UnsupportedLocation(String),
    /// The acquirer could not produce the package bytes.
    AcquireFailed(String),
    /// The supplied digest pin is not a `sha256:<hex>` string.
    InvalidDigest(String),
    /// The acquired bytes do not hash to the supplied pin.
    DigestMismatch(String),
    /// The bytes are not a raw wasm component, or failed validation.
    ArtifactRefused(String),
    /// The component exports no interface declared in the deployment's
    /// plugin seam list.
    SeamMissing(String),
    /// The package identity is already registered and cannot be re-bound.
    AlreadyActive(String),
    /// Loader misconfiguration or an internal registration failure.
    Internal(String),
}

impl fmt::Display for LoadError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UnsupportedLocation(reason)
            | Self::AcquireFailed(reason)
            | Self::InvalidDigest(reason)
            | Self::DigestMismatch(reason)
            | Self::ArtifactRefused(reason)
            | Self::SeamMissing(reason)
            | Self::AlreadyActive(reason)
            | Self::Internal(reason) => f.write_str(reason),
        }
    }
}

impl std::error::Error for LoadError {}

impl From<AdmitError> for LoadError {
    fn from(error: AdmitError) -> Self {
        match error {
            AdmitError::ArtifactRefused(reason) => Self::ArtifactRefused(reason),
            AdmitError::SeamMissing(reason) => Self::SeamMissing(reason),
            AdmitError::Internal(reason) => Self::Internal(reason),
        }
    }
}

/// Type-erased plugin loading, reachable from every store context.
///
/// Lives in the runtime's extensions so the `omnia:plugins/loader` host
/// binding invokes [`LoadPlugin::load_plugin`] without naming the concrete
/// runtime.
pub trait PluginLoader: Send + Sync + 'static {
    /// Load `package` (idempotent on package plus digest), returning its
    /// handle.
    fn load(
        &self, package: String, from: Location, digest: Option<String>,
    ) -> BoxFuture<'static, Result<Plugin, LoadError>>;
}

// The store-reachable loader. Non-owning: the extension lives inside the
// runtime's shared state, so a strong handle would leak it through a cycle.
struct RuntimeLoader<B: 'static> {
    runtime: WeakRuntime<B>,
}

impl<B: Clone + Send + Sync + 'static> PluginLoader for RuntimeLoader<B> {
    fn load(
        &self, package: String, from: Location, digest: Option<String>,
    ) -> BoxFuture<'static, Result<Plugin, LoadError>> {
        let runtime = self.runtime.clone();
        async move {
            let Some(runtime) = runtime.upgrade() else {
                return Err(LoadError::Internal("the runtime has shut down".to_owned()));
            };
            runtime.load_plugin(&package, from, digest.as_deref()).await
        }
        .boxed()
    }
}

/// The installed plugins extension: the deployment's acquisition policy, the
/// load serializer, the resolved digest per loader-loaded package, and the
/// store-reachable [`PluginLoader`].
pub struct Plugins {
    acquirer: Acquirer,
    // Serializes loads: they are rare, and serialization makes the
    // (package, digest) idempotency check race-free without single-flight
    // machinery.
    load_lock: tokio::sync::Mutex<()>,
    digests: Mutex<BTreeMap<GuestId, Arc<str>>>,
    loader: Arc<dyn PluginLoader>,
}

impl Plugins {
    /// Install the loader capability on `runtime`: the acquisition policy
    /// behind `omnia:plugins/loader.load` and [`LoadPlugin::load_plugin`],
    /// plus the store-reachable loader the `WasiPlugins` host binding serves.
    /// The [`Wiring::extend`](omnia_core::Wiring::extend) hook is the
    /// intended caller (the `runtime!` macro's `plugins: { locations: [...] }`
    /// list lowers into it).
    ///
    /// # Errors
    ///
    /// Returns an error if the capability is already installed.
    pub fn install<B>(runtime: &Runtime<B>, acquirer: Acquirer) -> anyhow::Result<()>
    where
        B: Clone + Send + Sync + 'static,
    {
        let plugins = Self {
            acquirer,
            load_lock: tokio::sync::Mutex::new(()),
            digests: Mutex::new(BTreeMap::new()),
            loader: Arc::new(RuntimeLoader {
                runtime: runtime.downgrade(),
            }),
        };
        anyhow::ensure!(
            runtime.extensions().insert(plugins),
            "the plugins capability installs exactly once per runtime"
        );
        Ok(())
    }

    pub(crate) fn loader(&self) -> Arc<dyn PluginLoader> {
        Arc::clone(&self.loader)
    }

    fn digest_of(&self, id: &GuestId) -> Option<Arc<str>> {
        self.digests.lock().unwrap_or_else(PoisonError::into_inner).get(id).cloned()
    }

    fn record(&self, id: GuestId, digest: Arc<str>) {
        let _ = self.digests.lock().unwrap_or_else(PoisonError::into_inner).insert(id, digest);
    }

    /// Drop the digest records of packages no longer registered, so a later
    /// re-load binds fresh bytes instead of comparing against a deregistered
    /// guest's.
    fn prune(&self, live: impl Fn(&GuestId) -> bool) {
        self.digests.lock().unwrap_or_else(PoisonError::into_inner).retain(|id, _| live(id));
    }
}

/// Late-bound plugin loading over a [`Runtime`] — the host side of the
/// `omnia:plugins/loader.load` capability.
pub trait LoadPlugin {
    /// Load a plugin: acquire bytes through the deployment's acquirer, verify
    /// the operator's sha256 pin, then admit the component (safe validation,
    /// seam check, registration) under `package`.
    ///
    /// Idempotent on (package, digest): an already-loaded package returns its
    /// handle immediately, and a conflicting pin for it refuses. A static
    /// deployment guest can never be re-bound.
    ///
    /// # Errors
    ///
    /// Returns a typed [`LoadError`] naming the refusal: unsupported
    /// location, acquisition failure, malformed or mismatched digest, refused
    /// artifact, missing seam export, identity conflict, or an internal
    /// registration failure.
    fn load_plugin<'a>(
        &'a self, package: &'a str, from: Location, pin: Option<&'a str>,
    ) -> impl Future<Output = Result<Plugin, LoadError>> + Send + 'a;
}

impl<B: Clone + Send + Sync + 'static> LoadPlugin for Runtime<B> {
    async fn load_plugin<'a>(
        &'a self, package: &'a str, from: Location, pin: Option<&'a str>,
    ) -> Result<Plugin, LoadError> {
        // A malformed pin is refused before any acquisition work.
        let pin =
            pin.map(digest::canonicalize_pin).transpose().map_err(LoadError::InvalidDigest)?;

        let Some(state) = self.extensions().get::<Plugins>() else {
            return Err(LoadError::Internal(format!(
                "this deployment has no acquirer; compile one in \
                 (`plugins: {{ locations: [...] }}`) to load `{package}`"
            )));
        };
        let _serial = state.load_lock.lock().await;

        // A deregistered package's digest record is stale; loads are rare and
        // serialized, so sweep here rather than hooking deregistration.
        state.prune(|id| self.registry().get(id).is_some());

        let id = GuestId::from(package);
        if self.registry().get(&id).is_some() {
            return match state.digest_of(&id) {
                Some(digest) if pin.as_deref().is_none_or(|pin| pin == &*digest) => {
                    Ok(Plugin { id, digest })
                }
                Some(digest) => Err(LoadError::AlreadyActive(format!(
                    "package `{package}` is already active with digest {digest}, which is not \
                     the requested pin"
                ))),
                None => Err(LoadError::AlreadyActive(format!(
                    "`{package}` is a deployment guest, which the loader cannot re-bind"
                ))),
            };
        }

        // Loads route structurally: each location kind reaches its acquirer
        // slot, and a kind with no slot refuses typed.
        let outcome = match &from {
            Location::Registry(endpoint) => match &state.acquirer.registry {
                Some(registry) => registry.acquire(package, endpoint.as_deref()).await,
                None => {
                    return Err(LoadError::UnsupportedLocation(format!(
                        "this deployment's locations serve no registry; loading `{package}` \
                         from {from} needs a `{{ registry: ... }}` entry"
                    )));
                }
            },
            Location::Path(path) => match &state.acquirer.path {
                Some(paths) => paths.acquire(path).await,
                None => {
                    return Err(LoadError::UnsupportedLocation(format!(
                        "this deployment's locations serve no paths; loading `{package}` \
                         from {from} needs a `{{ name: ..., path: ... }}` entry"
                    )));
                }
            },
        };
        let bytes = outcome.map_err(|error| {
            LoadError::AcquireFailed(format!("acquiring `{package}`: {error:#}"))
        })?;

        // The operator's pin binds name to bytes before any validation work.
        let resolved = crate::sha256_digest(&bytes);
        if let Some(pin) = &pin
            && *pin != resolved
        {
            return Err(LoadError::DigestMismatch(format!(
                "package `{package}` resolved to {resolved}, which is not the pinned {pin}"
            )));
        }

        // Validation, the seam check, and atomic publication are the
        // runtime's one privileged admission operation.
        self.admit(id.clone(), bytes).await?;

        let digest: Arc<str> = Arc::from(resolved);
        state.record(id.clone(), Arc::clone(&digest));
        tracing::debug!(package, digest = %digest, "plugin loaded");
        Ok(Plugin { id, digest })
    }
}
