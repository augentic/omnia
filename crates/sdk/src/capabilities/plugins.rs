//! Plugin-loading (requester) capability over `omnia:plugins/loader`.
//!
//! The requester surface for any application that late-binds guests into its
//! deployment: the guest names a [`Location`] — a guest the deployment
//! declares, a component beneath one of its read-only mounts, or a package
//! one of its registries serves — and the host acquires, verifies, and
//! admits the bytes it finds there, or attests the guest if it is already
//! active, handing back a typed [`Plugin`] handle. Component bytes never
//! cross the interface, and every location resolves inside what the
//! deployment granted.

use std::future::Future;
use std::path::Path;
use std::str::FromStr;

use serde::{Deserialize, Serialize};

/// Plugin loading (Omnia Plugins).
///
/// [`WasiProvider`](crate::WasiProvider) delegates to `omnia:plugins/loader`.
#[omnia_guest_macros::capability]
pub trait Plugins: Send + Sync {
    /// Ensure the guest `from` names is active and return its handle, held
    /// to `digest` when one is given; idempotent on (name, digest).
    ///
    /// # Errors
    ///
    /// Returns the loader's typed refusal ([`Error`]) when the deployment's
    /// grant does not serve the location, or the host cannot acquire,
    /// verify, validate, or register the bytes.
    fn load(
        &self, from: &Location, digest: Option<&Digest>,
    ) -> impl Future<Output = Result<Plugin, Error>> + Send;
}

/// A validated `sha256:<hex>` content digest, canonicalized to lowercase.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct Digest(String);

impl Digest {
    /// The canonical `sha256:<hex>` digest string.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl FromStr for Digest {
    type Err = Error;

    fn from_str(digest: &str) -> Result<Self, Error> {
        let Some(hex) = digest.strip_prefix(SCHEME) else {
            return Err(Error::Refused(format!(
                "digest `{digest}` does not use the `sha256:<hex>` scheme"
            )));
        };
        if hex.len() != HEX_LEN || !hex.chars().all(|c| c.is_ascii_hexdigit()) {
            return Err(Error::Refused(format!(
                "digest `{digest}` is not {HEX_LEN} hex characters"
            )));
        }
        Ok(Self(format!("{SCHEME}{}", hex.to_ascii_lowercase())))
    }
}

impl TryFrom<String> for Digest {
    type Error = Error;

    fn try_from(digest: String) -> Result<Self, Error> {
        digest.parse()
    }
}

impl From<Digest> for String {
    fn from(digest: Digest) -> Self {
        digest.0
    }
}

impl std::fmt::Display for Digest {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

const SCHEME: &str = "sha256:";
const HEX_LEN: usize = 64;

/// Where a load's component bytes come from, and the name the guest
/// registers under — the requester's mirror of the `omnia:plugins/loader`
/// `location` variant.
///
/// # Examples
///
/// ```
/// use omnia_sdk::plugins::Location;
///
/// assert_eq!(Location::Declared("intent".into()).name(), "intent");
/// assert_eq!(Location::Path("./adapters/intent.wasm".into()).name(), "intent");
/// let package = Location::Registry {
///     package: "emery:intent@1.0.0".into(),
///     endpoint: None,
/// };
/// assert_eq!(package.name(), "emery:intent");
/// ```
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Location {
    /// A guest the deployment declares, by name; it takes no digest, since
    /// the deployment's entry carries the pin.
    Declared(String),
    /// A component path beneath one of the deployment's read-only mounts,
    /// read fresh on every load. Registers as the path's file stem.
    Path(String),
    /// An exact `namespace:name@version` package, from the deployment's
    /// store or a package registry. Registers as the package reference
    /// without its version.
    Registry {
        /// The exact package reference to acquire: read from the
        /// deployment's store when it holds the release, fetched from a
        /// registry and written to the store once otherwise.
        package: String,
        /// The registry to fetch from when the deployment's `registries`
        /// routes the package's namespace nowhere; `None` takes that
        /// routing, and a namespace the deployment routes is fetched from
        /// its registry alone. A release the store holds is served from it
        /// whatever registry wrote it.
        endpoint: Option<String>,
    },
}

impl Location {
    /// The name a guest loaded from this location registers under.
    #[must_use]
    pub fn name(&self) -> &str {
        match self {
            Self::Declared(name) => name,
            Self::Path(path) => {
                Path::new(path).file_stem().and_then(|stem| stem.to_str()).unwrap_or(path)
            }
            Self::Registry { package, .. } => {
                package.split_once('@').map_or(package.as_str(), |(name, _)| name)
            }
        }
    }
}

// What the load named, for a refusal.
impl std::fmt::Display for Location {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Declared(name) => f.write_str(name),
            Self::Path(path) => f.write_str(path),
            Self::Registry { package, .. } => f.write_str(package),
        }
    }
}

/// A loaded plugin: the routed dispatch identity, its content digest, and
/// what it exports.
///
/// A plain value — loading confers no lifecycle authority over the loaded
/// component.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Plugin {
    id: String,
    digest: Digest,
    exports: Vec<String>,
}

impl Plugin {
    /// A handle over a routed identity, its content digest, and the names
    /// it exports — the constructor native suites use to script loads.
    #[must_use]
    pub fn new(
        id: impl Into<String>, digest: Digest, exports: impl IntoIterator<Item = impl Into<String>>,
    ) -> Self {
        Self {
            id: id.into(),
            digest,
            exports: exports.into_iter().map(Into::into).collect(),
        }
    }

    /// The routed identity host-mediated dispatch keys on.
    #[must_use]
    pub fn id(&self) -> &str {
        &self.id
    }

    /// The content digest of the bytes the guest was loaded from.
    #[must_use]
    pub const fn digest(&self) -> &Digest {
        &self.digest
    }

    /// The names the component exports: each interface by its full id
    /// (`acme:tool/run@1.0.0`), each bare function by name.
    #[must_use]
    pub fn exports(&self) -> &[String] {
        &self.exports
    }
}

/// Typed load refusal, mirroring the `omnia:plugins/loader` error variant.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum Error {
    /// The request or deployment is wrong and a retry cannot succeed: the
    /// deployment declares no guest of that name, the path is beneath no
    /// read-only mount, no registry routes the package, the bytes miss the
    /// digest, they are not a loadable component, they are pre-compiled
    /// where raw wasm alone is admitted (a caller-named path or package, a
    /// declared package, or a declared path without a `digest`), or the
    /// name is active under other bytes.
    #[error("refused: {0}")]
    Refused(String),
    /// The guest's source could not produce its bytes; the source may
    /// recover, so a retry can succeed.
    #[error("unavailable: {0}")]
    Unavailable(String),
    /// Loader misconfiguration or an internal registration failure.
    #[error("internal: {0}")]
    Internal(String),
}

impl Error {
    /// The kebab-case wire discriminant, stable for callers to branch on.
    #[must_use]
    pub const fn code(&self) -> &'static str {
        match self {
            Self::Refused(_) => "refused",
            Self::Unavailable(_) => "unavailable",
            Self::Internal(_) => "internal",
        }
    }
}

impl From<Error> for crate::Error {
    fn from(error: Error) -> Self {
        let code = error.code().to_owned();
        match error {
            Error::Unavailable(description) => Self::BadGateway { code, description },
            Error::Internal(description) => Self::ServerError { code, description },
            Error::Refused(description) => Self::BadRequest { code, description },
        }
    }
}
