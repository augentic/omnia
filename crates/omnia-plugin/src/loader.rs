//! The `omnia:plugins/loader` load path.

use std::collections::HashMap;
use std::future::Future;
use std::sync::Arc;

use anyhow::ensure;
use omnia_core::{AdmitError, Digest, GuestId, Runtime, Source, SourceSpec};

use crate::admission::{Admission, Registration};
use crate::error::LoadError;
use crate::source::RegistrySource;

/// Host-side `omnia:plugins/loader.load` — embedder sugar over the runtime's
/// installed [`Plugins`] extension.
pub trait PluginLoader {
    /// Ensure the guest the deployment declares as `name` is active and
    /// return its handle. Idempotent.
    ///
    /// # Errors
    ///
    /// `refused` on an undeclared name, a digest mismatch, or bytes that are
    /// not a loadable component; `unavailable` when the source could not
    /// produce the bytes; `internal` on registration failure.
    fn load(&self, name: &str) -> impl Future<Output = Result<Plugin, LoadError>> + Send;
}

impl<B: Clone + Send + Sync + 'static> PluginLoader for Runtime<B> {
    fn load(&self, name: &str) -> impl Future<Output = Result<Plugin, LoadError>> + Send {
        let plugins = self.extensions().get::<Plugins>();
        async move {
            match plugins {
                Some(plugins) => plugins.load(name).await,
                None => Err(LoadError::no_plugins(name)),
            }
        }
    }
}

/// The deployment's on-demand guest table over the runtime's admission seam.
///
/// The table is fixed at install: `load` admits a declared name from its
/// declared source, attests a name that is already active (declared at boot
/// or admitted earlier), and refuses every other name. Nothing a caller
/// passes chooses code.
pub struct Plugins {
    guests: HashMap<GuestId, Source>,
    registry: Arc<dyn RegistrySource>,
    admission: Box<dyn Admission>,
}

impl Plugins {
    /// Install the loader capability on `runtime`: `guests` is every source
    /// the deployment declares for on-demand loading, and `registry` fetches
    /// the [`SourceSpec::Package`] sources among them.
    ///
    /// # Errors
    ///
    /// Returns an error if an on-demand name repeats or is already registered
    /// at boot, or the capability is already installed.
    pub fn install<B>(
        runtime: &Runtime<B>, guests: impl IntoIterator<Item = Source>,
        registry: Arc<dyn RegistrySource>,
    ) -> anyhow::Result<()>
    where
        B: Clone + Send + Sync + 'static,
    {
        let mut table = HashMap::new();
        for source in guests {
            let id = source.id().clone();
            ensure!(
                runtime.registry().get(&id).is_none(),
                "on-demand guest `{id}` is already registered at boot"
            );
            ensure!(
                table.insert(id.clone(), source).is_none(),
                "on-demand guest `{id}` is declared twice"
            );
        }

        let plugins = Self {
            guests: table,
            registry,
            admission: Box::new(runtime.downgrade()),
        };
        ensure!(
            runtime.extensions().insert(plugins),
            "the plugins capability installs exactly once per runtime"
        );
        Ok(())
    }

    /// Ensure the guest the deployment declares as `name` is active and
    /// return its handle: acquired from its declared source, verified against
    /// what the source declares, and admitted through the runtime's admission
    /// seam if it loads on demand and is not active yet; attested with the
    /// digest the registry recorded otherwise. Idempotent.
    ///
    /// # Errors
    ///
    /// `refused` on an undeclared name, a digest mismatch, a pre-compiled
    /// artifact where the source admits raw wasm alone, or bytes that are not
    /// a loadable component; `unavailable` when the source could not produce
    /// the bytes; `internal` on registration failure.
    pub async fn load(&self, name: &str) -> Result<Plugin, LoadError> {
        let id = GuestId::from(name);
        if let Registration::Active(digest) = self.admission.registration(&id)? {
            return Ok(Plugin { id, digest });
        }
        let Some(source) = self.guests.get(&id) else {
            return Err(LoadError::Refused(format!(
                "no guest `{name}` is declared by this deployment"
            )));
        };

        let bytes = match source.spec() {
            SourceSpec::Package(package) => self.registry.acquire(package).await?,
            SourceSpec::Path(_) | SourceSpec::Bytes(_) => {
                source.read().await.map_err(|error| LoadError::Unavailable(format!("{error:#}")))?
            }
        };

        // What the source declares binds name to bytes before any validation work.
        let digest =
            source.verified(&bytes).map_err(|error| LoadError::Refused(format!("{error:#}")))?;

        match self.admission.admit(id.clone(), bytes, digest).await {
            Ok(()) => {
                tracing::debug!(%id, %digest, "guest loaded on demand");
                Ok(Plugin { id, digest })
            }
            // A racing load admitted the same declared entry first; the
            // winner's registration is the one to attest.
            Err(AdmitError::AlreadyRegistered(_)) => match self.admission.registration(&id)? {
                Registration::Active(digest) => Ok(Plugin { id, digest }),
                Registration::Absent => Err(LoadError::Internal(format!(
                    "`{name}` was admitted by a racing load and deregistered before it could be \
                     attested"
                ))),
            },
            Err(AdmitError::ArtifactRefused(reason)) => Err(LoadError::Refused(reason)),
            Err(AdmitError::Internal(reason)) => Err(LoadError::Internal(reason)),
        }
    }
}

/// Loaded plugin: routed identity plus content digest.
#[derive(Clone, Debug)]
pub struct Plugin {
    id: GuestId,
    digest: Digest,
}

impl Plugin {
    /// Routed identity for host-mediated dispatch.
    #[must_use]
    pub const fn id(&self) -> &GuestId {
        &self.id
    }

    /// The content digest of the bytes the guest was loaded from.
    #[must_use]
    pub const fn digest(&self) -> Digest {
        self.digest
    }
}
