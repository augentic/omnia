//! Identity wire types.

use std::fmt;
use std::time::Duration;

#[cfg(all(target_arch = "wasm32", target_os = "wasi"))]
use omnia_wasi_identity::credentials;

#[cfg(all(target_arch = "wasm32", target_os = "wasi"))]
use crate::Identity;
#[cfg(all(target_arch = "wasm32", target_os = "wasi"))]
use crate::capability::{Error, ErrorKind, Result};

/// The identity a provider's own [`Identity`](crate::Identity) handle uses.
pub const DEFAULT: &str = "default";

/// An `OAuth` 2.0 access token and how long it stays valid.
///
/// Its [`Debug`] form redacts the token.
#[derive(Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct AccessToken {
    /// The `OAuth` 2.0 access token.
    pub token: String,

    /// How long the token stays valid from when it was issued.
    pub expires_in: Duration,
}

impl AccessToken {
    /// A token `token` that stays valid for `expires_in`.
    pub fn new(token: impl Into<String>, expires_in: Duration) -> Self {
        Self {
            token: token.into(),
            expires_in,
        }
    }
}

impl fmt::Debug for AccessToken {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("AccessToken")
            .field("token", &"<redacted>")
            .field("expires_in", &self.expires_in)
            .finish()
    }
}

/// An [`Identity`](crate::Identity) handle on one identity of the WASI host.
///
/// [`Identity::named`](crate::Identity::named) on
/// [`WasiProvider`](crate::WasiProvider) returns one.
#[cfg(all(target_arch = "wasm32", target_os = "wasi"))]
#[derive(Clone, Debug)]
pub struct Handle {
    name: String,
}

#[cfg(all(target_arch = "wasm32", target_os = "wasi"))]
impl Handle {
    pub(crate) fn new(name: &str) -> Self {
        Self {
            name: name.to_owned(),
        }
    }
}

#[cfg(all(target_arch = "wasm32", target_os = "wasi"))]
impl Identity for Handle {
    type Named = Self;

    async fn token(&self, scopes: &[&str]) -> Result<AccessToken> {
        let identity = credentials::get_identity(self.name.clone()).await?;
        let scopes = scopes.iter().map(|&scope| scope.to_owned()).collect();
        let token = identity.get_token(scopes).await?;
        Ok(AccessToken::new(token.token, Duration::from_secs(token.expires_in)))
    }

    fn named(&self, name: &str) -> Self {
        Self::new(name)
    }
}

#[cfg(all(target_arch = "wasm32", target_os = "wasi"))]
impl From<credentials::Error> for Error {
    fn from(err: credentials::Error) -> Self {
        match err {
            credentials::Error::NoSuchIdentity => {
                Self::new(ErrorKind::NotFound, "no such identity")
            }
            credentials::Error::InternalFailure(message) => Self::new(ErrorKind::Other, message),
        }
    }
}
