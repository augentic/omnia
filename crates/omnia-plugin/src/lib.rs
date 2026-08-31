//! # Plugin acquisition
//!
//! The [`Acquirer`] seam and its built-in acquirers.
//!
//! Acquisition policy — endpoints, cache, path reads — is a value the
//! embedder compiles in at the composition root (the `runtime!` macro's
//! `plugins: { locations: [...] }` list, lowered into the generated
//! `Wiring::acquirer` hook), never runtime-core machinery: the runtime
//! routes each load to the [`Acquirer`] slot for its [`Location`] kind and
//! keeps zero storage and network dependencies. This crate is omnia-internal
//! — its surface reaches consumers re-exported from `omnia` under the
//! runtime's own paths. Store implementors depend on this crate for
//! [`ContentStore`] and [`ReleaseStore`].

mod path;
mod registry;
mod store;

use std::fmt;
use std::sync::Arc;

use anyhow::Result;
use futures::future::BoxFuture;

pub use self::path::PathAcquire;
pub use self::registry::RegistryAcquire;
pub use self::store::{
    ContentStore, NoStore, PluginStore, ReleaseRecord, ReleaseStore, sha256_digest,
};

/// Where an acquirer finds a package's component bytes — the mirror of the
/// `omnia:plugins/loader` `location` variant.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Location {
    /// A package registry; `None` selects the acquirer's default.
    Registry(Option<String>),
    /// A preopen-relative component path, read fresh on every load.
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

/// Acquisition policy compiled in at the composition root, one slot per
/// [`Location`] kind: a load routes structurally, and a kind with no slot
/// refuses typed.
///
/// Built by the runtime's `Wiring::acquirer` hook, which the `runtime!`
/// macro's `plugins: { locations: [...] }` list lowers into. A slot owns
/// every fetch, cache, and endpoint decision; the loader only ever receives
/// bytes back, then verifies, validates, and registers them host-side.
#[derive(Clone, Default)]
pub struct Acquirer {
    /// Serves [`Location::Registry`] loads; `None` refuses the kind.
    pub registry: Option<Arc<dyn AcquireRegistry>>,
    /// Serves [`Location::Path`] loads; `None` refuses the kind.
    pub path: Option<Arc<dyn AcquirePath>>,
}

/// Registry acquisition policy — the [`Acquirer::registry`] slot.
pub trait AcquireRegistry: Send + Sync + 'static {
    /// Produce the raw component bytes for `package` from `registry`
    /// (`None` selects the acquirer's default endpoint).
    fn acquire<'a>(
        &'a self, package: &'a str, registry: Option<&'a str>,
    ) -> BoxFuture<'a, Result<Vec<u8>>>;
}

/// Path acquisition policy — the [`Acquirer::path`] slot.
pub trait AcquirePath: Send + Sync + 'static {
    /// Produce the raw component bytes at the location-relative `path`.
    fn acquire<'a>(&'a self, path: &'a str) -> BoxFuture<'a, Result<Vec<u8>>>;
}
