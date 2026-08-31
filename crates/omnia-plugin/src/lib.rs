//! # Plugin acquisition
//!
//! The [`Acquirer`] seam behind the `omnia:plugins/loader` capability, plus
//! its built-in acquirers. An `Acquirer` is a value the embedder compiles in
//! at the composition root — one slot per [`Location`] kind — so the runtime
//! core keeps zero storage and network dependencies. Store implementors
//! depend on this crate for [`ContentStore`] and [`ReleaseStore`].

mod path;
mod registry;
mod store;

use std::fmt;
use std::sync::Arc;

pub use self::path::{AcquirePath, PathAcquire};
pub use self::registry::{AcquireRegistry, RegistryAcquire};
pub use self::store::{
    ContentStore, NoStore, PluginStore, ReleaseRecord, ReleaseStore, sha256_digest,
};

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

/// Compiled-in acquisition policy: one slot per [`Location`] kind, where an
/// empty slot refuses the kind.
#[derive(Clone, Default)]
pub struct Acquirer {
    /// Serves [`Location::Registry`] loads.
    pub registry: Option<Arc<dyn AcquireRegistry>>,
    /// Serves [`Location::Path`] loads.
    pub path: Option<Arc<dyn AcquirePath>>,
}
