//! Where one load's bytes come from — the host mirror of the
//! `omnia:plugins/loader` `location` variant — and the registry seam that
//! fetches a package.

use std::fmt;

use futures::future::BoxFuture;
use omnia_core::GuestId;

use crate::error::LoadError;

/// Where a load's component bytes come from, and the name the guest
/// registers under.
///
/// Every location resolves inside what the deployment granted: its guest
/// list for a declared name, its read-only mounts for a path, and its
/// `registries` routing for a package.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Location {
    /// A guest the deployment declares, by name.
    Declared(String),
    /// A component path beneath one of the deployment's read-only mounts.
    /// Registers as the path's file stem.
    Path(String),
    /// An exact `namespace:name@version` from a package registry. Registers
    /// as the package reference.
    Registry {
        /// The exact package reference to fetch.
        package: String,
        /// The registry to fetch from when the deployment's routing does not
        /// claim the package's namespace.
        endpoint: Option<String>,
    },
}

impl Location {
    /// The name a guest loaded from this location registers under.
    #[must_use]
    pub fn id(&self) -> GuestId {
        match self {
            Self::Declared(name) => GuestId::from(name.as_str()),
            Self::Path(path) => GuestId::from_path(path),
            Self::Registry { package, .. } => GuestId::from(package.as_str()),
        }
    }
}

// What the load named, for a refusal.
impl fmt::Display for Location {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Declared(name) => f.write_str(name),
            Self::Path(path) => f.write_str(path),
            Self::Registry { package, .. } => f.write_str(package),
        }
    }
}

/// Registry acquisition policy — how a [`SourceSpec::Package`] source is
/// fetched.
///
/// [`SourceSpec::Package`]: omnia_core::SourceSpec::Package
pub trait RegistrySource: Send + Sync + 'static {
    /// Produce the raw component bytes for the exact `package` reference —
    /// from `endpoint` when the load names one and the deployment's routing
    /// does not claim the package's namespace, else from that routing —
    /// split by remedy: [`LoadError::Refused`] for an authoritative "no"
    /// (nothing routes it, a routed namespace named another registry, the
    /// registry has no such release), never for a source failure a retry
    /// might clear ([`LoadError::Unavailable`]).
    fn acquire<'a>(
        &'a self, package: &'a str, endpoint: Option<&'a str>,
    ) -> BoxFuture<'a, Result<Vec<u8>, LoadError>>;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ids() {
        let registry = Location::Registry {
            package: "acme:tool@1.0.0".to_owned(),
            endpoint: Some("ghcr.io".to_owned()),
        };
        assert_eq!(registry.id(), GuestId::from("acme:tool@1.0.0"));
        assert_eq!(Location::Path("./adapters/tool.wasm".to_owned()).id(), GuestId::from("tool"));
        assert_eq!(Location::Declared("tool".to_owned()).id(), GuestId::from("tool"));
    }
}
