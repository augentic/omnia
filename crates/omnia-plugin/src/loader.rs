//! The `omnia:plugins/loader` load path.

use std::future::Future;
use std::sync::Arc;

use omnia_core::{AdmitError, GuestId, Runtime, sha256_digest};

use crate::admission::{Admission, Registration};
use crate::error::LoadError;
use crate::source::{Origin, PathSource, RegistrySource};

/// Host-side `omnia:plugins/loader.load` — embedder sugar over the runtime's
/// installed [`Plugins`] extension.
pub trait PluginLoader {
    /// Acquire, pin-check, and admit the guest `from` names. Idempotent on
    /// (name, digest).
    ///
    /// # Errors
    ///
    /// `refused` on a bad request, an undeclared name, or a pin,
    /// `unavailable` on acquisition failure, `already-active` on an identity
    /// conflict, `internal` on registration failure.
    fn load(
        &self, from: Origin, pin: Option<&str>,
    ) -> impl Future<Output = Result<Plugin, LoadError>> + Send;
}

impl<B: Clone + Send + Sync + 'static> PluginLoader for Runtime<B> {
    fn load(
        &self, from: Origin, pin: Option<&str>,
    ) -> impl Future<Output = Result<Plugin, LoadError>> + Send {
        let plugins = self.extensions().get::<Plugins>();
        async move {
            match plugins {
                Some(plugins) => plugins.load(from, pin).await,
                None => Err(LoadError::no_plugins(from.label())),
            }
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
    /// Install the loader capability on `runtime` over a custom policy.
    ///
    /// `registry` and `path` are the acquisition slots, one per acquiring
    /// [`Origin`] kind; `None` refuses that kind. The declared policy —
    /// [`install_declared`](Self::install_declared) — fills them from the
    /// deployment's mounts and `registries` configuration, and is what
    /// assembly installs unless an embedder selects custom sources. A
    /// declared name acquires nothing and is answered from the registry
    /// under either policy.
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

    /// Acquire, pin-check, and admit the guest `from` names through the
    /// runtime's admission seam, registering it under the name the origin
    /// derives ([`Origin::id`]). Idempotent on (name, digest). A declared
    /// name acquires nothing: the handle attests the registration, carrying
    /// whatever digest it recorded.
    ///
    /// # Errors
    ///
    /// `refused` on a bad request, an undeclared name, or a pin,
    /// `unavailable` on acquisition failure, `already-active` on an identity
    /// conflict, `internal` on registration failure.
    pub async fn load(&self, from: Origin, pin: Option<&str>) -> Result<Plugin, LoadError> {
        let pin = pin.map(canonicalize).transpose().map_err(LoadError::Refused)?;
        let id = from.id();

        if let Origin::Declared(name) = &from {
            if pin.is_some() {
                return Err(LoadError::Refused(format!(
                    "`{name}` is declared by the deployment; it takes no pin"
                )));
            }
            return match self.admission.registration(&id)? {
                Registration::Active(recorded) => Ok(Plugin {
                    id,
                    digest: recorded.map(Arc::from),
                }),
                Registration::Absent => Err(LoadError::Refused(format!(
                    "no guest `{name}` is declared by this deployment"
                ))),
            };
        }

        let label = from.label();
        if let Registration::Active(recorded) = self.admission.registration(&id)? {
            return attest_active(label, id, recorded.as_deref(), pin.as_deref());
        }

        let bytes = self.acquire(&from).await?;

        // The operator's pin binds name to bytes before any validation work.
        let hash = sha256_digest(&bytes);
        if pin.is_some_and(|pin| pin != hash) {
            return Err(LoadError::Refused(format!(
                "resolved `{label}` digest {hash} does not match the pinned digest"
            )));
        }

        match self.admission.admit(id.clone(), bytes).await {
            Ok(()) => {
                tracing::debug!(%id, "plugin loaded");
                Ok(Plugin {
                    id,
                    digest: Some(Arc::from(hash)),
                })
            }
            Err(AdmitError::AlreadyRegistered(_)) => {
                let recorded = self.admission.registration(&id)?.digest();
                attest_active(label, id, recorded.as_deref(), Some(&hash))
            }
            Err(error) => Err(error.into()),
        }
    }

    async fn acquire(&self, from: &Origin) -> Result<Vec<u8>, LoadError> {
        match from {
            Origin::Registry { package, endpoint } => match &self.registry {
                Some(registry) => registry.acquire(package, endpoint.as_deref()).await,
                None => Err(LoadError::Refused(format!(
                    "this deployment refuses registry loads; `{package}` cannot be fetched"
                ))),
            },
            Origin::Path(path) => match &self.path {
                Some(paths) => paths.acquire(path).await,
                None => Err(LoadError::Refused(format!(
                    "this deployment mounts no directories; loading `{path}` needs a `mounts:` \
                     entry"
                ))),
            },
            Origin::Declared(name) => {
                Err(LoadError::Internal(format!("declared guest `{name}` reached acquisition")))
            }
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

/// Attest an active registration as the requested (name, digest), or
/// refuse: an active identity never re-binds.
fn attest_active(
    label: &str, id: GuestId, recorded: Option<&str>, wanted: Option<&str>,
) -> Result<Plugin, LoadError> {
    match recorded {
        Some(digest) if wanted == Some(digest) => Ok(Plugin {
            id,
            digest: Some(Arc::from(digest)),
        }),
        _ => Err(LoadError::AlreadyActive(format!("`{label}` is already active"))),
    }
}

/// Loaded plugin: routed identity plus content digest.
#[derive(Clone, Debug)]
pub struct Plugin {
    id: GuestId,
    digest: Option<Arc<str>>,
}

impl Plugin {
    /// Routed identity for host-mediated dispatch.
    #[must_use]
    pub const fn id(&self) -> &GuestId {
        &self.id
    }

    /// Resolved `sha256:<hex>` of the loaded bytes; `None` for a declared
    /// guest the registry recorded no digest for.
    #[must_use]
    pub fn digest(&self) -> Option<&str> {
        self.digest.as_deref()
    }
}
