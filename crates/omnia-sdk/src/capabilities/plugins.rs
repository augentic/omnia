//! Plugin-loading (requester) capability over `omnia:plugins/loader`.
//!
//! The requester surface for any application that late-binds guests into its
//! deployment: the guest names one of the deployment's declared guests, and
//! the host admits it from the source the deployment declares — or attests
//! it if it is already active — handing back a typed [`Plugin`] handle.
//! Nothing the requester passes chooses code: no bytes, paths, or registry
//! endpoints cross the interface.

use std::future::Future;
use std::str::FromStr;

use serde::{Deserialize, Serialize};

/// Bindings for the `omnia:plugins` imports world.
#[cfg(target_arch = "wasm32")]
mod generated {
    #![allow(missing_docs)]

    wit_bindgen::generate!({
        world: "imports",
        path: "wit",
        generate_all,
    });
}

/// The canonical digest scheme prefix.
const SCHEME: &str = "sha256:";

/// Hex characters in a sha256 digest.
const HEX_LEN: usize = 64;

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

/// A loaded plugin: the routed dispatch identity plus its content digest.
///
/// A plain value — loading confers no lifecycle authority over the loaded
/// component.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Plugin {
    id: String,
    digest: Option<Digest>,
}

impl Plugin {
    /// A handle over a routed identity and its content digest — the
    /// constructor native suites use to script loads.
    #[must_use]
    pub fn new(id: impl Into<String>, digest: Option<Digest>) -> Self {
        Self {
            id: id.into(),
            digest,
        }
    }

    /// The routed identity host-mediated dispatch keys on.
    #[must_use]
    pub fn id(&self) -> &str {
        &self.id
    }

    /// The content digest of the guest's bytes; `None` for a guest whose
    /// bytes the runtime never hashed.
    #[must_use]
    pub const fn digest(&self) -> Option<&Digest> {
        self.digest.as_ref()
    }
}

/// Typed load refusal, mirroring the `omnia:plugins/loader` error variant.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum Error {
    /// The request or deployment is wrong and a retry cannot succeed: the
    /// deployment declares no guest of that name, its bytes miss the
    /// declared digest, or they are not a valid raw component.
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

/// Plugin loading (Omnia Plugins).
///
/// The default WASM implementation delegates to `omnia:plugins/loader`; off
/// `wasm32` the signature is bare so native suites script loads.
pub trait Plugins: Send + Sync {
    /// Ensure the guest the deployment declares as `name` is active and
    /// return its handle; idempotent.
    ///
    /// # Errors
    ///
    /// Returns the loader's typed refusal ([`Error`]) when the deployment
    /// declares no such guest, or the host cannot acquire, verify, validate,
    /// or register it.
    #[cfg(not(target_arch = "wasm32"))]
    fn load(&self, name: &str) -> impl Future<Output = Result<Plugin, Error>> + Send;

    /// Ensure the guest the deployment declares as `name` is active and
    /// return its handle; idempotent.
    ///
    /// # Errors
    ///
    /// Returns the loader's typed refusal ([`Error`]) when the deployment
    /// declares no such guest, or the host cannot acquire, verify, validate,
    /// or register it.
    #[cfg(target_arch = "wasm32")]
    fn load(&self, name: &str) -> impl Future<Output = Result<Plugin, Error>> + Send {
        use generated::omnia::plugins::loader;

        let name = name.to_owned();
        async move {
            let loaded = loader::load(name).await?;
            let digest = loaded.digest.map(|digest| digest.parse()).transpose().map_err(
                |error: Error| {
                    Error::Internal(format!("host reported a malformed digest: {error}"))
                },
            )?;
            Ok(Plugin::new(loaded.id, digest))
        }
    }
}

delegate_deref!(Plugins {
    fn load(&self, name: &str) -> impl Future<Output = Result<Plugin, Error>> + Send {
        (**self).load(name)
    }
});

/// The WASI-backed provider a `wasm32` guest hands its wasm-free core; the
/// default method body carries the whole delegation.
#[cfg(target_arch = "wasm32")]
#[derive(Clone, Copy, Debug)]
pub struct WasiPlugins;

#[cfg(target_arch = "wasm32")]
impl Plugins for WasiPlugins {}

/// Wire conversion from the `omnia:plugins/loader` refusal variant.
#[cfg(target_arch = "wasm32")]
mod wire {
    use super::Error;
    use super::generated::omnia::plugins::loader;

    impl From<loader::Error> for Error {
        fn from(error: loader::Error) -> Self {
            match error {
                loader::Error::Refused(detail) => Self::Refused(detail),
                loader::Error::Unavailable(detail) => Self::Unavailable(detail),
                loader::Error::Internal(detail) => Self::Internal(detail),
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{Digest, Error};

    #[test]
    fn digest_canonicalizes_case() {
        let parsed: Digest =
            format!("sha256:{}", "AB".repeat(32)).parse().expect("uppercase hex accepted");
        assert_eq!(parsed.as_str(), format!("sha256:{}", "ab".repeat(32)));
    }

    #[test]
    fn digest_malformed() {
        for digest in [
            format!("sha512:{}", "ab".repeat(32)),
            "sha256:abcd".into(),
            format!("sha256:{}", "zz".repeat(32)),
        ] {
            let error = digest.parse::<Digest>().expect_err("malformed digest refused");
            assert!(matches!(error, Error::Refused(_)), "{digest} refused");
        }
    }

    #[test]
    fn digest_serde() {
        let json = format!("\"sha256:{}\"", "ab".repeat(32));
        let parsed: Digest = serde_json::from_str(&json).expect("valid digest deserializes");
        assert_eq!(serde_json::to_string(&parsed).expect("serializes"), json);
        serde_json::from_str::<Digest>("\"sha256:abcd\"").expect_err("malformed digest refused");
    }

    #[test]
    fn taxonomy_mapping() {
        let cases = [
            (Error::Refused("r".into()), "refused"),
            (Error::Unavailable("u".into()), "unavailable"),
            (Error::Internal("x".into()), "internal"),
        ];
        for (error, code) in cases {
            let mapped = crate::Error::from(error.clone());
            assert_eq!(mapped.code(), code);
            match error {
                Error::Unavailable(_) => {
                    assert!(matches!(mapped, crate::Error::BadGateway { .. }));
                }
                Error::Internal(_) => {
                    assert!(matches!(mapped, crate::Error::ServerError { .. }));
                }
                Error::Refused(_) => assert!(matches!(mapped, crate::Error::BadRequest { .. })),
            }
        }
    }
}
