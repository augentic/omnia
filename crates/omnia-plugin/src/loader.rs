//! The `omnia:plugins/loader` load path.

use std::future::Future;
use std::sync::Arc;

use futures::FutureExt as _;
use futures::future::BoxFuture;
use omnia_core::{AdmitError, GuestId, Location, Runtime, WeakRuntime, sha256_digest};

use crate::Origin;
use crate::host::Error;
use crate::path::{PathMounts, PathSource};
use crate::registry::{RegistryClient, RegistrySource};

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
        &self, package: &str, from: Origin, pin: Option<&str>,
    ) -> impl Future<Output = Result<Plugin, Error>> + Send;
}

impl<B: Clone + Send + Sync + 'static> PluginLoader for Runtime<B> {
    fn load(
        &self, package: &str, from: Origin, pin: Option<&str>,
    ) -> impl Future<Output = Result<Plugin, Error>> + Send {
        let plugins = self.extensions().get::<Plugins>();
        async move {
            match plugins {
                Some(plugins) => plugins.load(package, from, pin).await,
                None => Err(Error::Internal(format!(
                    "this deployment has no plugins; compile one in (`plugins: {{ locations: [...] }}`) to load `{package}`"
                ))),
            }
        }
    }
}

/// The privileged runtime half the loader drives — the registry record and
/// the admission seam — erased of the deployment's backend type so
/// [`Plugins`] can live in the runtime's extensions.
trait Admission: Send + Sync + 'static {
    /// The registration state of `id`, with any recorded digest.
    fn registration(&self, id: &GuestId) -> Result<Registration, Error>;

    /// Admit raw wasm bytes as the late guest `id`.
    fn admit(&self, id: GuestId, bytes: Vec<u8>) -> BoxFuture<'static, Result<(), AdmitError>>;
}

// Weak: a strong handle would cycle through the extension.
impl<B: Clone + Send + Sync + 'static> Admission for WeakRuntime<B> {
    fn registration(&self, id: &GuestId) -> Result<Registration, Error> {
        let runtime = self
            .upgrade()
            .ok_or_else(|| Error::Internal("the runtime has shut down".to_owned()))?;
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

enum Registration {
    Absent,
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

/// Installed acquisition policy over the runtime's admission seam.
pub struct Plugins {
    registry: Option<Arc<dyn RegistrySource>>,
    path: Option<Arc<dyn PathSource>>,
    admission: Box<dyn Admission>,
}

impl Plugins {
    /// Install the loader capability on `runtime`.
    ///
    /// `registry` and `path` are the compiled-in slots, one per [`Origin`]
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

    /// Install the loader capability over the deployment's declared
    /// locations ([`Runtime::plugin_locations`]): every path entry folds, in
    /// declaration order, into one [`PathMounts`] filling the path slot, the
    /// registry entry into a cacheless [`RegistryClient`] filling the
    /// registry slot. A deployment declaring no locations installs nothing,
    /// so a load refuses as loader misconfiguration.
    ///
    /// # Errors
    ///
    /// Returns an error if a path location cannot be opened or the capability
    /// is already installed.
    pub fn install_declared<B>(runtime: &Runtime<B>) -> anyhow::Result<()>
    where
        B: Clone + Send + Sync + 'static,
    {
        let locations = runtime.plugin_locations();
        if locations.is_empty() {
            return Ok(());
        }
        let paths: Vec<(&str, &std::path::Path)> = locations
            .iter()
            .filter_map(|location| match location {
                Location::Path { name, path } => Some((name.as_str(), path.as_path())),
                Location::Registry { .. } => None,
            })
            .collect();
        let path: Option<Arc<dyn PathSource>> =
            if paths.is_empty() { None } else { Some(Arc::new(PathMounts::new(paths)?)) };
        let registry: Option<Arc<dyn RegistrySource>> =
            locations.iter().find_map(|location| match location {
                Location::Registry { registry } => {
                    Some(Arc::new(RegistryClient::new(registry.as_str())) as Arc<dyn RegistrySource>)
                }
                Location::Path { .. } => None,
            });
        Self::install(runtime, registry, path)
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
        &self, package: &str, from: Origin, pin: Option<&str>,
    ) -> Result<Plugin, Error> {
        let pin = pin.map(canonicalize).transpose().map_err(Error::Refused)?;

        let id = GuestId::from(package);
        if let Registration::Active(recorded) = self.admission.registration(&id)? {
            return attest_active(package, id, recorded.as_deref(), pin.as_deref());
        }

        let bytes = self.acquire(package, &from).await?;

        // The operator's pin binds name to bytes before any validation work.
        let hash = sha256_digest(&bytes);
        if pin.is_some_and(|pin| pin != hash) {
            return Err(Error::Refused(format!(
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

    async fn acquire(&self, package: &str, from: &Origin) -> Result<Vec<u8>, Error> {
        match from {
            Origin::Registry(endpoint) => match &self.registry {
                Some(registry) => registry.acquire(package, endpoint.as_deref()).await,
                None => Err(Error::Refused(format!(
                    "this deployment's locations serve no registry; loading `{package}` needs \
                     a `{{ registry: ... }}` entry"
                ))),
            },
            Origin::Path(path) => match &self.path {
                Some(paths) => paths.acquire(path).await,
                None => Err(Error::Refused(format!(
                    "this deployment's locations serve no paths; loading `{package}` from \
                     `{path}` needs a `{{ name: ..., path: ... }}` entry"
                ))),
            },
        }
    }
}

const SCHEME: &str = "sha256:";
const HEX_LEN: usize = 64;

// Canonicalize a digest so it can be compared.
fn canonicalize(digest: &str) -> Result<String, String> {
    let Some(hex) = digest.strip_prefix(SCHEME) else {
        return Err(format!("digest `{digest}` is not `{SCHEME}<hex>`"));
    };
    if hex.len() != HEX_LEN || !hex.chars().all(|c| c.is_ascii_hexdigit()) {
        return Err(format!("digest `{digest}` is not {HEX_LEN} hex characters"));
    }
    Ok(format!("{SCHEME}{}", hex.to_ascii_lowercase()))
}

/// Attest an active registration as the requested (package, digest), or
/// refuse: an active identity never re-binds.
fn attest_active(
    package: &str, id: GuestId, recorded: Option<&str>, wanted: Option<&str>,
) -> Result<Plugin, Error> {
    match recorded {
        Some(digest) if wanted == Some(digest) => Ok(Plugin {
            id,
            digest: Arc::from(digest),
        }),
        _ => Err(Error::AlreadyActive(format!("`{package}` is already active"))),
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

impl From<AdmitError> for Error {
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
