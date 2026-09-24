//! Where an on-demand guest's bytes come from, and the registry seam that
//! fetches a package source.

use std::borrow::Cow;
use std::fmt;
use std::path::PathBuf;

use futures::future::BoxFuture;
use omnia_core::Digest;

use crate::error::LoadError;

/// Where an on-demand guest's bytes come from, as its `[[guest]]` entry
/// declares them.
#[derive(Clone)]
pub enum Origin {
    /// A component file, read fresh on every admission.
    Path(PathBuf),
    /// Component bytes compiled into the host binary.
    Bytes(Cow<'static, [u8]>),
    /// An exact `namespace:name@version`, fetched from the registry the
    /// deployment's `registries` configuration routes it to.
    Package(String),
}

// Manual: the derived impl would dump the embedded component bytes.
impl fmt::Debug for Origin {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Path(path) => f.debug_tuple("Path").field(path).finish(),
            Self::Bytes(bytes) => write!(f, "Bytes({} bytes)", bytes.len()),
            Self::Package(package) => f.debug_tuple("Package").field(package).finish(),
        }
    }
}

/// A guest the deployment declares for on-demand loading: where its bytes
/// come from, and the digest they must hash to.
#[derive(Clone, Debug)]
pub struct OnDemand {
    /// Where the bytes come from.
    pub origin: Origin,
    /// The digest the bytes must hash to; unpinned when `None`.
    pub digest: Option<Digest>,
}

/// Registry acquisition policy — how an [`Origin::Package`] source is
/// fetched.
pub trait RegistrySource: Send + Sync + 'static {
    /// Produce the raw component bytes for the exact `package` reference,
    /// split by remedy: [`LoadError::Refused`] for an authoritative "no" (no
    /// registry routes it, the registry has no such release), never for a
    /// source failure a retry might clear ([`LoadError::Unavailable`]).
    fn acquire<'a>(&'a self, package: &'a str) -> BoxFuture<'a, Result<Vec<u8>, LoadError>>;
}
