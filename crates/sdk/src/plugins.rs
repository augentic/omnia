//! `omnia:plugins`, the guests a guest late-binds into its deployment.
//!
//! A guest names a [`Location`]:
//!
//! - a guest the deployment declares
//! - a component beneath one of its read-only mounts
//! - a package one of its registries serves
//!
//! [`Location::load`] has the host acquire, verify, and admit the bytes it
//! finds there, or attest the guest when it is already active, and answers
//! a [`Plugin`] handle. Component bytes never cross the interface, and every
//! location resolves inside what the deployment granted.

use std::path::Path;
use std::str::FromStr;

use serde::{Deserialize, Serialize};

pub use crate::bindings::imports::omnia::plugins::{loader, types};

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
/// registers under.
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
    /// Ensure the guest this location names is active and return its handle,
    /// held to `digest` when one is given.
    ///
    /// A load is idempotent on the name and digest.
    ///
    /// # Errors
    ///
    /// The loader's typed refusal, an [`Error`], when the deployment's grant
    /// does not serve the location, or the host cannot acquire, verify,
    /// validate, or register the bytes.
    pub async fn load(&self, digest: Option<&Digest>) -> Result<Plugin, Error> {
        match loader::load(self.source(digest)?).await? {
            Ok(loaded) => {
                let digest = loaded.digest.parse().map_err(|error: Error| {
                    Error::Internal(format!("host reported a malformed digest: {error}"))
                })?;
                Ok(Plugin::new(loaded.name, digest, loaded.exports))
            }
            Err(conflict) => Err(Error::Refused(self.conflicted(digest, &conflict))),
        }
    }

    // the deployment's entry pins a declared guest, so its source carries no pin
    fn source(&self, pin: Option<&Digest>) -> Result<types::Source, Error> {
        let pin = pin.map(ToString::to_string);
        let source = match self {
            Self::Declared(name) if pin.is_some() => {
                return Err(Error::Refused(format!(
                    "`{name}` is declared by the deployment, which pins it; the load takes no digest"
                )));
            }
            Self::Declared(name) => types::Source::Declared(name.clone()),
            Self::Path(path) => types::Source::Path(types::File {
                path: path.clone(),
                pin,
            }),
            Self::Registry { package, endpoint } => types::Source::Release(types::Release {
                reference: package.clone(),
                registry: endpoint.clone(),
                pin,
            }),
        };
        Ok(source)
    }

    // the bytes found and what they missed: the load's pin, or the bytes
    // already active under the name
    fn conflicted(&self, pin: Option<&Digest>, conflict: &types::Conflict) -> String {
        let found = &conflict.found;
        match (pin, &conflict.current) {
            (Some(pin), _) if pin.as_str() != found => {
                format!("`{self}` resolved to {found}, not the pinned {pin}")
            }
            (_, Some(current)) => {
                format!("`{}` is loaded from {}, not {found}", current.name, current.digest)
            }
            (_, None) => format!("`{}` was active under bytes other than {found}", self.name()),
        }
    }

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

/// Why a load failed, with the host's six `omnia:plugins` cases folded onto
/// three codes.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum Error {
    /// The request or the deployment is wrong, and a retry cannot succeed.
    ///
    /// The host's `not-found`, `denied`, and `invalid` cases fold here, and
    /// so does a load that conflicted:
    ///
    /// - the deployment declares no guest of that name, or no file or
    ///   release is where the location points;
    /// - the path is beneath no read-only mount, no registry routes the
    ///   package, or the location derives a name the deployment declares;
    /// - the bytes miss the digest, are not a loadable component, or are
    ///   pre-compiled where raw wasm alone is admitted (a caller-named path
    ///   or package, a declared package, or a declared path without a
    ///   `digest`);
    /// - the name is active under other bytes.
    #[error("refused: {0}")]
    Refused(String),
    /// The guest's source could not produce its bytes, the host's
    /// `unavailable` case.
    ///
    /// The source may recover, so a retry can succeed.
    #[error("unavailable: {0}")]
    Unavailable(String),
    /// The host cannot serve the load, its `unsupported` and `other` cases.
    ///
    /// The runtime has no loader or no registry client, or the registration
    /// failed.
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

// a retry cannot mend what the request or the grant got wrong, so those
// cases refuse
impl From<types::Error> for Error {
    fn from(error: types::Error) -> Self {
        match error {
            types::Error::NotFound(detail)
            | types::Error::Denied(detail)
            | types::Error::Invalid(detail) => Self::Refused(detail),
            types::Error::Unavailable(detail) => Self::Unavailable(detail),
            types::Error::Unsupported(detail) | types::Error::Other(detail) => {
                Self::Internal(detail)
            }
        }
    }
}
