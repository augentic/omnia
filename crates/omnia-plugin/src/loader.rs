//! The `omnia:plugins/loader` load path.

use std::future::Future;
use std::sync::Arc;

use futures::FutureExt as _;
use futures::future::BoxFuture;
use omnia_core::{AdmitError, GuestId, Runtime, WeakRuntime, sha256_digest};

use crate::host::Error as LoadError;
use crate::path::PathSource;
use crate::registry::RegistrySource;
use crate::{Location, digest};

/// Host-side `omnia:plugins/loader.load` — embedder sugar over the runtime's
/// installed [`Plugins`] extension.
pub trait PluginLoader {
    /// Acquire, pin-check, and admit `package`. Idempotent on (package, digest).
    ///
    /// # Errors
    ///
    /// `refused` on a bad request or pin, `unavailable` on acquisition failure,
    /// `already-active` on an identity conflict, `internal` on registration
    /// failure.
    fn load(
        &self, package: &str, from: Location, pin: Option<&str>,
    ) -> impl Future<Output = Result<Plugin, LoadError>> + Send;
}

impl<B: Clone + Send + Sync + 'static> PluginLoader for Runtime<B> {
    fn load(
        &self, package: &str, from: Location, pin: Option<&str>,
    ) -> impl Future<Output = Result<Plugin, LoadError>> + Send {
        let plugins = self.extensions().get::<Plugins>();
        async move {
            match plugins {
                Some(plugins) => plugins.load(package, from, pin).await,
                None => Err(missing_plugins(package)),
            }
        }
    }
}

/// The refusal for a deployment that never installed [`Plugins`].
pub fn missing_plugins(package: &str) -> LoadError {
    LoadError::Internal(format!(
        "this deployment has no plugins; compile one in (`plugins: {{ locations: [...] }}`) to \
         load `{package}`"
    ))
}

/// A guest identity's registration state at load time.
enum Registration {
    /// No guest holds the identity.
    Absent,
    /// A guest holds the identity, with the digest recorded at admission.
    Active(Option<String>),
}

impl Registration {
    /// The recorded digest, when active with one.
    fn digest(self) -> Option<String> {
        match self {
            Self::Active(digest) => digest,
            Self::Absent => None,
        }
    }
}

/// The privileged runtime half the loader drives — the registry record and
/// the admission seam — erased of the deployment's backend type so
/// [`Plugins`] can live in the runtime's extensions.
trait Admission: Send + Sync + 'static {
    /// The registration state of `id`, with any recorded digest.
    fn registration(&self, id: &GuestId) -> Result<Registration, LoadError>;

    /// Admit raw wasm bytes as the late guest `id`.
    fn admit(&self, id: GuestId, bytes: Vec<u8>) -> BoxFuture<'static, Result<(), AdmitError>>;
}

// Weak: a strong handle would cycle through the extension.
impl<B: Clone + Send + Sync + 'static> Admission for WeakRuntime<B> {
    fn registration(&self, id: &GuestId) -> Result<Registration, LoadError> {
        let runtime = self
            .upgrade()
            .ok_or_else(|| LoadError::Internal("the runtime has shut down".to_owned()))?;
        Ok(runtime.registry().get(id).map_or(Registration::Absent, |guest| {
            Registration::Active(guest.digest().map(str::to_owned))
        }))
    }

    fn admit(&self, id: GuestId, bytes: Vec<u8>) -> BoxFuture<'static, Result<(), AdmitError>> {
        let weak = self.clone();
        async move {
            let Some(runtime) = weak.upgrade() else {
                return Err(AdmitError::Internal("the runtime has shut down".to_owned()));
            };
            runtime.admit(id, bytes).await
        }
        .boxed()
    }
}

/// Installed acquisition policy over the runtime's admission seam.
pub struct Plugins {
    registry: Option<Arc<dyn RegistrySource>>,
    path: Option<Arc<dyn PathSource>>,
    admission: Box<dyn Admission>,
}

impl Plugins {
    /// Install the loader capability on `runtime`.
    ///
    /// `registry` and `path` are the compiled-in slots, one per [`Location`]
    /// kind; `None` refuses that kind.
    ///
    /// # Errors
    ///
    /// Returns an error if the capability is already installed.
    pub fn install<B>(
        runtime: &Runtime<B>, registry: Option<Arc<dyn RegistrySource>>,
        path: Option<Arc<dyn PathSource>>,
    ) -> anyhow::Result<()>
    where
        B: Clone + Send + Sync + 'static,
    {
        let plugins = Self {
            registry,
            path,
            admission: Box::new(runtime.downgrade()),
        };

        anyhow::ensure!(
            runtime.extensions().insert(plugins),
            "the plugins capability installs exactly once per runtime"
        );
        Ok(())
    }

    /// Acquire, pin-check, and admit `package` through the runtime's
    /// admission seam. Idempotent on (package, digest).
    ///
    /// # Errors
    ///
    /// `refused` on a bad request or pin, `unavailable` on acquisition
    /// failure, `already-active` on an identity conflict, `internal` on
    /// registration failure.
    pub async fn load(
        &self, package: &str, from: Location, pin: Option<&str>,
    ) -> Result<Plugin, LoadError> {
        let pin = pin.map(digest::canonicalize).transpose().map_err(LoadError::Refused)?;

        let id = GuestId::from(package);
        if let Registration::Active(recorded) = self.admission.registration(&id)? {
            return attest_active(package, id, recorded.as_deref(), pin.as_deref());
        }

        let bytes = self.acquire(package, &from).await?;

        // The operator's pin binds name to bytes before any validation work.
        let hash = sha256_digest(&bytes);
        if pin.is_some_and(|pin| pin != hash) {
            return Err(LoadError::Refused(format!(
                "resolved package `{package}` digest {hash} does not match the pinned digest"
            )));
        }

        match self.admission.admit(id.clone(), bytes).await {
            Ok(()) => {
                tracing::debug!(package, "plugin loaded");
                Ok(Plugin {
                    id,
                    digest: Arc::from(hash),
                })
            }
            Err(AdmitError::AlreadyRegistered(_)) => {
                let recorded = self.admission.registration(&id)?.digest();
                attest_active(package, id, recorded.as_deref(), Some(&hash))
            }
            Err(error) => Err(error.into()),
        }
    }

    async fn acquire(&self, package: &str, from: &Location) -> Result<Vec<u8>, LoadError> {
        match from {
            Location::Registry(endpoint) => match &self.registry {
                Some(registry) => registry.acquire(package, endpoint.as_deref()).await,
                None => Err(LoadError::Refused(format!(
                    "this deployment's locations serve no registry; loading `{package}` needs \
                     a `{{ registry: ... }}` entry"
                ))),
            },
            Location::Path(path) => match &self.path {
                Some(paths) => paths.acquire(path).await,
                None => Err(LoadError::Refused(format!(
                    "this deployment's locations serve no paths; loading `{package}` from \
                     `{path}` needs a `{{ name: ..., path: ... }}` entry"
                ))),
            },
        }
    }
}

/// Attest an active registration as the requested (package, digest), or
/// refuse: an active identity never re-binds.
fn attest_active(
    package: &str, id: GuestId, recorded: Option<&str>, wanted: Option<&str>,
) -> Result<Plugin, LoadError> {
    match recorded {
        Some(digest) if wanted == Some(digest) => Ok(Plugin {
            id,
            digest: Arc::from(digest),
        }),
        _ => Err(LoadError::AlreadyActive(format!("`{package}` is already active"))),
    }
}

/// Loaded plugin: routed identity plus content digest.
#[derive(Clone, Debug)]
pub struct Plugin {
    id: GuestId,
    digest: Arc<str>,
}

impl Plugin {
    /// Routed identity for host-mediated dispatch.
    #[must_use]
    pub const fn id(&self) -> &GuestId {
        &self.id
    }

    /// Resolved `sha256:<hex>` of the loaded bytes.
    #[must_use]
    pub fn digest(&self) -> &str {
        &self.digest
    }
}

impl From<AdmitError> for LoadError {
    fn from(error: AdmitError) -> Self {
        match error {
            AdmitError::ArtifactRefused(reason) | AdmitError::SeamMissing(reason) => {
                Self::Refused(reason)
            }
            AdmitError::AlreadyRegistered(reason) => Self::AlreadyActive(reason),
            AdmitError::Internal(reason) => Self::Internal(reason),
        }
    }
}
