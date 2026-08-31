//! # Late-bound plugins
//!
//! The `omnia:plugins/loader` host capability: a guest names code (package,
//! location, optional sha256 pin) and the host acquires, verifies, validates,
//! and registers it, handing back a typed [`Plugin`] handle. Component bytes
//! never cross the interface in either direction, and the requester receives
//! no lifecycle authority — validation, compilation, and publication stay
//! host-side, bounded by the deployment's declared plugin interfaces.
//!
//! Acquisition policy (endpoints, cache, path reads) is the embedder's
//! [`Acquire`] value, built by the [`Wiring::acquirer`](crate::Wiring::acquirer)
//! hook (the `runtime!` macro's `plugins: { acquire: ... }` key lowers into
//! it) once the deployment's backends have connected. Core ships
//! [`MountAcquire`] — preopen-relative reads over the mount registry — and
//! keeps zero storage and network dependencies.

mod acquire;
mod digest;
mod host;

use std::collections::BTreeMap;
use std::fmt;
use std::sync::{Arc, Mutex, OnceLock, PoisonError};

pub use acquire::{Acquire, AcquireContext, AcquireError, Location, MountAcquire};
use futures::FutureExt as _;
use futures::future::BoxFuture;
pub use host::{WasiPlugins, WasiPluginsCtxView, WasiPluginsView};
use wasmtime::component::{Component, types};

use crate::deployment::{ELF_MAGIC, GuestArtifact};
use crate::dispatch::serve_guest;
use crate::registry::{Guest, GuestId};
use crate::runtime::{Runtime, RuntimeDispatcher};

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

/// Type-erased plugin loading, threaded into store contexts beside the
/// dispatcher so the `omnia:plugins/loader` host binding reaches
/// [`Runtime::load_plugin`] without naming the concrete runtime.
pub trait PluginLoader: Send + Sync + 'static {
    /// Load `package` (idempotent on package plus digest), returning its
    /// handle.
    fn load(
        &self, package: String, from: Location, digest: Option<String>,
    ) -> BoxFuture<'static, Result<Plugin, LoadError>>;
}

impl<B: Clone + Send + Sync + 'static> PluginLoader for RuntimeDispatcher<B> {
    fn load(
        &self, package: String, from: Location, digest: Option<String>,
    ) -> BoxFuture<'static, Result<Plugin, LoadError>> {
        let runtime = self.runtime();
        async move { runtime.load_plugin(&package, from, digest.as_deref()).await }.boxed()
    }
}

/// Runtime-held loader state: the installed acquirer, the load serializer,
/// and the resolved digest per loader-loaded package.
pub struct PluginsState {
    acquirer: OnceLock<Arc<dyn Acquire>>,
    // Serializes loads: they are rare, and serialization makes the
    // (package, digest) idempotency check race-free without single-flight
    // machinery.
    load_lock: tokio::sync::Mutex<()>,
    digests: Mutex<BTreeMap<GuestId, Arc<str>>>,
}

impl PluginsState {
    pub(crate) const fn new() -> Self {
        Self {
            acquirer: OnceLock::new(),
            load_lock: tokio::sync::Mutex::const_new(()),
            digests: Mutex::new(BTreeMap::new()),
        }
    }

    pub(crate) fn install_acquirer(&self, acquirer: Arc<dyn Acquire>) {
        if self.acquirer.set(acquirer).is_err() {
            tracing::warn!("acquirer already installed; ignoring");
        }
    }

    /// Drop the digest record of a deregistered package so a later re-load
    /// binds fresh bytes instead of comparing against the removed guest's.
    pub(crate) fn forget(&self, id: &GuestId) {
        let _ = self.digests.lock().unwrap_or_else(PoisonError::into_inner).remove(id);
    }

    fn digest_of(&self, id: &GuestId) -> Option<Arc<str>> {
        self.digests.lock().unwrap_or_else(PoisonError::into_inner).get(id).cloned()
    }

    fn record(&self, id: GuestId, digest: Arc<str>) {
        let _ = self.digests.lock().unwrap_or_else(PoisonError::into_inner).insert(id, digest);
    }
}

impl<B: Clone + Send + Sync + 'static> Runtime<B> {
    /// Load a plugin: acquire bytes through the deployment's acquirer, verify
    /// the operator's sha256 pin, validate raw wasm, check the component
    /// exports a declared plugin seam, and register it under `package` — the
    /// host side of the `omnia:plugins/loader.load` capability.
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
    pub async fn load_plugin(
        &self, package: &str, from: Location, pin: Option<&str>,
    ) -> Result<Plugin, LoadError> {
        // A malformed pin is refused before any acquisition work.
        let pin =
            pin.map(digest::canonicalize_pin).transpose().map_err(LoadError::InvalidDigest)?;

        let state = self.plugins_state();
        let _serial = state.load_lock.lock().await;

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

        let Some(acquirer) = state.acquirer.get() else {
            return Err(LoadError::Internal(format!(
                "this deployment has no acquirer; compile one in (`plugins: {{ acquire: ... }}`) \
                 to load `{package}`"
            )));
        };
        let context = AcquireContext {
            mounts: self.mount_registry(),
        };
        let bytes =
            acquirer.acquire(package, &from, &context).await.map_err(|error| match error {
                AcquireError::Unsupported(reason) => LoadError::UnsupportedLocation(reason),
                AcquireError::Failed(error) => {
                    LoadError::AcquireFailed(format!("acquiring `{package}`: {error:#}"))
                }
            })?;

        // The operator's pin binds name to bytes before any validation work.
        let resolved = digest::sha256_hex(&bytes);
        if let Some(pin) = &pin
            && *pin != resolved
        {
            return Err(LoadError::DigestMismatch(format!(
                "package `{package}` resolved to {resolved}, which is not the pinned {pin}"
            )));
        }

        // A native (pre-compiled) artifact is refused before wasmtime sees
        // the bytes: loader results only ever take the safe validation path,
        // never the deployment's `Precompiled` trust policy.
        if bytes.get(..ELF_MAGIC.len()) == Some(&ELF_MAGIC) {
            return Err(LoadError::ArtifactRefused(format!(
                "package `{package}` is a pre-compiled (native) artifact; the loader only \
                 accepts raw wasm components"
            )));
        }

        // Safe validation plus sandboxed JIT — the explicitly safe constructor.
        let component =
            GuestArtifact::wasm(bytes).load(self.registry().engine()).await.map_err(|error| {
                LoadError::ArtifactRefused(format!("validating `{package}`: {error:#}"))
            })?;

        self.check_seam_export(package, &component)?;

        // The same publish sequence as `Runtime::register`: pre-instantiate
        // against the shared host set, wire seam exports, publish atomically.
        let instance_pre = self.registry().instantiate_late(&id, &component).map_err(|error| {
            LoadError::ArtifactRefused(format!("pre-instantiating `{package}`: {error:#}"))
        })?;
        let guest = Guest::local(id.clone(), instance_pre);
        let endpoint = serve_guest(self, &guest).await.map_err(|error| {
            LoadError::Internal(format!("serving `{package}` seam exports: {error:#}"))
        })?;
        self.registry()
            .publish(guest, endpoint)
            .map_err(|error| LoadError::Internal(format!("publishing `{package}`: {error:#}")))?;

        let digest: Arc<str> = Arc::from(resolved);
        state.record(id.clone(), Arc::clone(&digest));
        tracing::debug!(package, digest = %digest, "plugin loaded");
        Ok(Plugin { id, digest })
    }

    /// Refuse a component that exports no interface from the deployment's
    /// declared plugin seam list.
    fn check_seam_export(&self, package: &str, component: &Component) -> Result<(), LoadError> {
        let links = self.registry().dispatch().links();
        if links.is_empty() {
            return Err(LoadError::SeamMissing(format!(
                "cannot load `{package}`: this deployment declares no plugin interfaces"
            )));
        }
        let engine = self.registry().engine();
        let exports_seam =
            component.component_type().exports(engine).any(|(interface, extern_)| {
                links.contains(interface)
                    && matches!(extern_.ty, types::ComponentItem::ComponentInstance(_))
            });
        if exports_seam {
            Ok(())
        } else {
            let declared: Vec<&str> = links.iter().map(AsRef::as_ref).collect();
            Err(LoadError::SeamMissing(format!(
                "package `{package}` exports none of the declared plugin interfaces ({})",
                declared.join(", ")
            )))
        }
    }
}
