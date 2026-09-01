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

/// Host-side `omnia:plugins/loader.load`.
pub trait LoadPlugin {
    /// Acquire, pin-check, and admit `package`. Idempotent on (package, digest).
    ///
    /// # Errors
    ///
    /// `refused` on a bad request or pin, `unavailable` on acquisition failure,
    /// `already-active` on an identity conflict, `internal` on registration
    /// failure.
    fn load_plugin(
        &self, package: &str, from: Location, pin: Option<&str>,
    ) -> impl Future<Output = Result<Plugin, LoadError>> + Send;
}

impl<B: Clone + Send + Sync + 'static> LoadPlugin for Runtime<B> {
    async fn load_plugin(
        &self, package: &str, from: Location, pin: Option<&str>,
    ) -> Result<Plugin, LoadError> {
        let pin = pin.map(digest::canonicalize).transpose().map_err(LoadError::Refused)?;
        let Some(plugins) = self.extensions().get::<Plugins>() else {
            return Err(LoadError::Internal(format!(
                "this deployment has no plugins; compile one in (`plugins: {{ locations: [...] }}`) to load `{package}`"
            )));
        };

        // return the plugin if the package is already loaded
        let id = GuestId::from(package);
        if let Some(guest) = self.registry().get(&id) {
            return already_active(package, id, guest.digest(), pin.as_deref());
        }

        // get the package bytes from the specified location
        let bytes = plugins.load(package, &from).await?;

        // the operator's pin binds name to bytes before any validation work
        let hash = sha256_digest(&bytes);
        if pin.is_some_and(|pin| pin != hash) {
            return Err(LoadError::Refused(format!(
                "resolved package `{package}` digest {hash} does not match the pinned digest"
            )));
        }

        // admit the wasm guest into the runtime
        match self.admit(id.clone(), bytes).await {
            Ok(()) => {
                tracing::debug!(package, "plugin loaded");
                Ok(Plugin {
                    id,
                    digest: Arc::from(hash),
                })
            }
            Err(AdmitError::AlreadyRegistered(_)) => {
                let recorded =
                    self.registry().get(&id).and_then(|guest| guest.digest().map(str::to_owned));
                already_active(package, id, recorded.as_deref(), Some(&hash))
            }
            Err(error) => Err(error.into()),
        }
    }
}

fn already_active(
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

/// Installed acquisition policy and store-reachable loader.
pub struct Plugins {
    registry: Option<Arc<dyn RegistrySource>>,
    path: Option<Arc<dyn PathSource>>,
    loader: Arc<dyn PluginLoader>,
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
            loader: Arc::new(runtime.downgrade()),
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

    async fn load(&self, package: &str, from_loc: &Location) -> Result<Vec<u8>, LoadError> {
        match from_loc {
            Location::Registry(endpoint) => match &self.registry {
                Some(registry) => registry.acquire(package, endpoint.as_deref()).await,
                None => Err(LoadError::Refused(format!(
                    "this deployment's locations serve no registry; loading `{package}` \
                     from {from_loc} needs a `{{ registry: ... }}` entry"
                ))),
            },
            Location::Path(path) => match &self.path {
                Some(paths) => paths.acquire(path).await,
                None => Err(LoadError::Refused(format!(
                    "this deployment's locations serve no paths; loading `{package}` \
                     from {from_loc} needs a `{{ name: ..., path: ... }}` entry"
                ))),
            },
        }
    }
}

/// Type-erased load so the host binding need not name the runtime.
pub trait PluginLoader: Send + Sync + 'static {
    /// Load `package`, idempotent on package plus digest.
    fn load(
        &self, package: String, from: Location, digest: Option<String>,
    ) -> BoxFuture<'static, Result<Plugin, LoadError>>;
}

// Weak: a strong handle would cycle through the extension.
impl<B: Clone + Send + Sync + 'static> PluginLoader for WeakRuntime<B> {
    fn load(
        &self, package: String, from: Location, digest: Option<String>,
    ) -> BoxFuture<'static, Result<Plugin, LoadError>> {
        let runtime = self.clone();
        async move {
            let Some(runtime) = runtime.upgrade() else {
                return Err(LoadError::Internal("the runtime has shut down".to_owned()));
            };
            runtime.load_plugin(&package, from, digest.as_deref()).await
        }
        .boxed()
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
