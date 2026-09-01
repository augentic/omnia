//! The `omnia:plugins/loader` load path.

use std::collections::BTreeMap;
use std::future::Future;
use std::sync::Arc;

use futures::FutureExt as _;
use futures::future::BoxFuture;
use omnia_core::{AdmitError, GuestId, Runtime, WeakRuntime};

use crate::host::Error as LoadError;
use crate::{Acquirer, Location, digest};

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
        let Some(loaded) = self.extensions().get::<Plugins>() else {
            return Err(LoadError::Internal(no_acquirer(package)));
        };

        let mut digests = loaded.digests.lock().await;
        digests.retain(|id, _| self.registry().get(id).is_some());

        // return the plugin if the package is already loaded
        let id = GuestId::from(package);
        if self.registry().get(&id).is_some() {
            return match digests.get(&id) {
                Some(digest) if pin.as_deref() == Some(digest.as_ref()) => Ok(Plugin {
                    id,
                    digest: Arc::clone(digest),
                }),
                _ => Err(LoadError::AlreadyActive(format!("`{package}` is already active"))),
            };
        }

        // get the package bytes from the specified location
        let bytes = loaded.acquirer.acquire(package, &from).await?;

        // the operator's pin binds name to bytes before any validation work
        let hash = crate::sha256_digest(&bytes);
        if pin.is_some_and(|pin| pin != hash) {
            return Err(LoadError::Refused(format!(
                "resolved package `{package}` digest {hash} does not match the pinned digest"
            )));
        }

        // load the guest into the runtime
        self.admit(id.clone(), bytes).await?;

        // insert the digest into the registry
        let digest = Arc::from(hash);
        digests.insert(id.clone(), Arc::clone(&digest));
        drop(digests);

        tracing::debug!(package, "plugin loaded");
        Ok(Plugin { id, digest })
    }
}

/// Installed acquisition policy, digest records, and store-reachable loader.
pub struct Plugins {
    acquirer: Acquirer,
    // Serializes loads so the (package, digest) check is race-free.
    digests: tokio::sync::Mutex<BTreeMap<GuestId, Arc<str>>>,
    loader: Arc<dyn PluginLoader>,
}

impl Plugins {
    /// Install the loader capability on `runtime`.
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
            digests: tokio::sync::Mutex::new(BTreeMap::new()),
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
            AdmitError::Internal(reason) => Self::Internal(reason),
        }
    }
}

/// Refusal when the deployment installed no [`Plugins`] extension.
pub fn no_acquirer(package: &str) -> String {
    format!(
        "this deployment has no acquirer; compile one in (`plugins: {{ locations: [...] }}`) to \
         load `{package}`"
    )
}
