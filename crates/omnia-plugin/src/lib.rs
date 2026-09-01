//! # Late-bound plugins
//!
//! The `omnia:plugins/loader` capability crate: a guest names code (package,
//! location, optional sha256 pin) and the host acquires, verifies, and admits
//! it through the runtime's admission seam, handing back a typed [`Plugin`]
//! handle. Component bytes never cross the interface in either direction, and
//! the requester receives no lifecycle authority — validation, compilation,
//! and publication stay host-side, bounded by the deployment's declared
//! plugin interfaces.
//!
//! Everything plugin lives here: the [`WasiPlugins`] host binding, the
//! [`LoadPlugin`] load path, and the acquisition seam. Acquisition policy
//! (endpoints, cache, path reads) is the embedder's [`Acquirer`] value — one
//! slot per [`Location`] kind — installed by [`Plugins::install`] from the
//! deployment's [`Wiring::extend`](omnia_core::Wiring::extend) hook (the
//! `runtime!` macro's `plugins: { locations: [...] }` list lowers into it).
//! The built-in acquirers are [`PathMounts`] and [`RegistryClient`]; store
//! implementors depend on this crate for [`ContentStore`] and
//! [`ReleaseStore`]. The runtime core keeps zero storage and network
//! dependencies.

mod digest;
mod host;
mod loader;
mod path;
mod registry;
mod store;

use std::fmt;
use std::sync::Arc;

pub use omnia_core::sha256_digest;

pub use self::host::{Error as LoadError, WasiPlugins, WasiPluginsCtxView, WasiPluginsView};
pub use self::loader::{LoadPlugin, Plugin, PluginLoader, Plugins};
pub use self::path::{PathMounts, PathSource};
pub use self::registry::{RegistryClient, RegistrySource};
pub use self::store::{ContentStore, NoStore, PluginStore, ReleaseRecord, ReleaseStore};

/// Where an acquirer finds a package's component bytes — the mirror of the
/// `omnia:plugins/loader` `location` variant.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Location {
    /// A package registry; `None` selects the acquirer's default.
    Registry(Option<String>),
    /// A location-relative component path.
    Path(String),
}

impl fmt::Display for Location {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Registry(None) => f.write_str("the default registry"),
            Self::Registry(Some(registry)) => write!(f, "registry `{registry}`"),
            Self::Path(path) => write!(f, "path `{path}`"),
        }
    }
}

/// Why an acquirer could not produce a package's bytes, split by remedy.
///
/// A [`Refused`](Self::Refused) request can never succeed as written; an
/// [`Unavailable`](Self::Unavailable) source may recover.
#[derive(Debug)]
pub enum AcquireError {
    /// An authoritative refusal — a malformed reference, a package or path
    /// the source does not serve; retrying the same request cannot succeed.
    Refused(anyhow::Error),
    /// The source failed to produce the bytes but may recover, so a retry
    /// can succeed.
    Unavailable(anyhow::Error),
}

impl fmt::Display for AcquireError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let (Self::Refused(error) | Self::Unavailable(error)) = self;
        // `:#` keeps the full context chain from acquisition errors.
        if f.alternate() { write!(f, "{error:#}") } else { write!(f, "{error}") }
    }
}

impl std::error::Error for AcquireError {}

/// Compiled-in acquisition policy: one slot per [`Location`] kind, where an
/// empty slot refuses the kind.
#[derive(Clone, Default)]
pub struct Acquirer {
    /// Serves [`Location::Registry`] loads.
    pub registry: Option<Arc<dyn RegistrySource>>,
    /// Serves [`Location::Path`] loads.
    pub path: Option<Arc<dyn PathSource>>,
}

impl Acquirer {
    // Get the package bytes from the specified location.
    async fn acquire(&self, package: &str, from_loc: &Location) -> Result<Vec<u8>, LoadError> {
        let outcome = match from_loc {
            Location::Registry(endpoint) => match &self.registry {
                Some(registry) => registry.acquire(package, endpoint.as_deref()).await,
                None => {
                    return Err(LoadError::Refused(format!(
                        "this deployment's locations serve no registry; loading `{package}` \
                         from {from_loc} needs a `{{ registry: ... }}` entry"
                    )));
                }
            },
            Location::Path(path) => match &self.path {
                Some(paths) => paths.acquire(path).await,
                None => {
                    return Err(LoadError::Refused(format!(
                        "this deployment's locations serve no paths; loading `{package}` \
                         from {from_loc} needs a `{{ name: ..., path: ... }}` entry"
                    )));
                }
            },
        };

        outcome.map_err(|error| match error {
            AcquireError::Refused(error) => {
                LoadError::Refused(format!("acquiring `{package}`: {error:#}"))
            }
            AcquireError::Unavailable(error) => {
                LoadError::Unavailable(format!("acquiring `{package}`: {error:#}"))
            }
        })
    }
}
