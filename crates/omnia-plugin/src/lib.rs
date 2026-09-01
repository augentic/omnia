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
//! (endpoints, cache, path reads) is the two slots [`Plugins::install`]
//! takes — one per [`Location`] kind — from the deployment's
//! [`Wiring::extend`](omnia_core::Wiring::extend) hook (the `runtime!`
//! macro's `plugins: { locations: [...] }` list lowers into it). The
//! built-in acquirers are [`PathMounts`] and [`RegistryClient`]; store
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
