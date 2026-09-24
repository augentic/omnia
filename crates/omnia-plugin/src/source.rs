//! The acquisition seam: where one load's bytes come from and the two
//! per-kind acquirer slots [`Plugins`](crate::Plugins) fills.

use futures::future::BoxFuture;
use omnia_core::GuestId;

use crate::error::LoadError;

/// Where one load's component bytes come from, the host mirror of the
/// `omnia:plugins/loader` `location` variant.
///
/// Each origin derives the name the guest registers under ([`Origin::id`])
/// and resolves against the deployment's declared policy: its mounts for a
/// path, its `registries` configuration for a package the load routes past
/// no endpoint of its own, and its own guest set for a declared name.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Origin {
    /// An exact `namespace:name@version` from a package registry —
    /// `endpoint` when the load names one, else the configuration's routing.
    /// Registers as the package reference.
    Registry {
        /// The exact package reference to fetch.
        package: String,
        /// The registry to fetch from, when the load names one.
        endpoint: Option<String>,
    },
    /// A mount-relative component path. Registers as the path's file stem.
    Path(String),
    /// A guest the deployment declares, by name. Nothing is acquired.
    Declared(String),
}

impl Origin {
    /// The name a guest loaded from this origin registers under.
    #[must_use]
    pub fn id(&self) -> GuestId {
        match self {
            Self::Registry { package, .. } => GuestId::from(package.as_str()),
            Self::Path(path) => GuestId::from_path(path),
            Self::Declared(name) => GuestId::from(name.as_str()),
        }
    }

    /// What the load named, for a refusal.
    #[must_use]
    pub fn label(&self) -> &str {
        match self {
            Self::Registry { package, .. } => package,
            Self::Path(path) => path,
            Self::Declared(name) => name,
        }
    }
}

/// Path acquisition policy — the path slot of [`Plugins`](crate::Plugins).
pub trait PathSource: Send + Sync + 'static {
    /// Produce the raw component bytes at the mount-relative `path`, split
    /// by remedy: [`LoadError::Refused`] for a path no mount serves, never
    /// for a read failure a retry might clear ([`LoadError::Unavailable`]).
    fn acquire<'a>(&'a self, path: &'a str) -> BoxFuture<'a, Result<Vec<u8>, LoadError>>;
}

/// Registry acquisition policy — the registry slot of
/// [`Plugins`](crate::Plugins).
pub trait RegistrySource: Send + Sync + 'static {
    /// Produce the raw component bytes for `package` from `registry`
    /// (`None` lets the acquirer's configuration route the package), split
    /// by remedy: [`LoadError::Refused`] for an authoritative "no", never
    /// for a source failure a retry might clear ([`LoadError::Unavailable`]).
    fn acquire<'a>(
        &'a self, package: &'a str, registry: Option<&'a str>,
    ) -> BoxFuture<'a, Result<Vec<u8>, LoadError>>;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ids() {
        let registry = Origin::Registry {
            package: "acme:tool@1.0.0".to_owned(),
            endpoint: Some("ghcr.io".to_owned()),
        };
        assert_eq!(registry.id(), GuestId::from("acme:tool@1.0.0"));
        assert_eq!(Origin::Path("./adapters/tool.wasm".to_owned()).id(), GuestId::from("tool"));
        assert_eq!(Origin::Path("tool.cwasm".to_owned()).id(), GuestId::from("tool"));
        assert_eq!(Origin::Declared("tool".to_owned()).id(), GuestId::from("tool"));
    }
}
