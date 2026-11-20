//! `omnia:identity`, access tokens for the identities a deployment declares.
//!
//! A guest opens an [`Identity`] with [`open`], [`DEFAULT`] unless the
//! deployment declares another. Beside the generated `get_token`, an
//! identity returns an [`AccessToken`] whose [`Debug`] form redacts the
//! token.

use std::fmt;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use crate::Result;
pub use crate::bindings::imports::omnia::identity::{credentials, types};
pub use crate::identity::credentials::Identity;

/// The identity a guest opens when the deployment names no other.
pub const DEFAULT: &str = "default";

/// Open the identity `name`, a binding the deployment declares.
///
/// # Errors
///
/// The host's failure, under its code: `not-found` for an identity the
/// deployment does not declare.
pub async fn open(name: &str) -> Result<Identity> {
    Ok(credentials::open(name.to_owned()).await?)
}

impl Identity {
    /// An access token for this identity, scoped to `scopes`.
    ///
    /// An empty `scopes` asks for the identity's default scopes. A portable
    /// request names one scope.
    ///
    /// # Errors
    ///
    /// The host's failure, under its code: `unsupported` for a number of
    /// scopes the backend cannot serve.
    pub async fn token(&self, scopes: &[&str]) -> Result<AccessToken> {
        let scopes = scopes.iter().map(|&scope| scope.to_owned()).collect();
        let token = self.get_token(scopes).await?;
        Ok(AccessToken::new(token.token, instant(token.expires_at)))
    }

    /// The token string for this identity's default scopes.
    ///
    /// # Errors
    ///
    /// The host's failure, under its code: `unsupported` when the identity
    /// has no default scopes.
    pub async fn access_token(&self) -> Result<String> {
        Ok(self.token(&[]).await?.token)
    }
}

// `millis` since the Unix epoch, held below the latest instant every
// platform's `SystemTime` can represent
fn instant(millis: u64) -> SystemTime {
    UNIX_EPOCH + Duration::from_millis(millis).min(Duration::from_secs(u64::from(u32::MAX)))
}

/// An `OAuth` 2.0 bearer token and when it expires.
///
/// Its [`Debug`] form redacts the token.
#[derive(Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct AccessToken {
    /// The token, which a caller sends in an `Authorization: Bearer` header.
    pub token: String,

    /// When the token expires.
    pub expires_at: SystemTime,
}

impl AccessToken {
    /// A token `token` that expires at `expires_at`.
    pub fn new(token: impl Into<String>, expires_at: SystemTime) -> Self {
        Self {
            token: token.into(),
            expires_at,
        }
    }

    /// How long the token stays valid from now, zero once it has expired.
    #[must_use]
    pub fn expires_in(&self) -> Duration {
        self.expires_at.duration_since(SystemTime::now()).unwrap_or_default()
    }
}

impl fmt::Debug for AccessToken {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("AccessToken")
            .field("token", &"<redacted>")
            .field("expires_at", &self.expires_at)
            .finish()
    }
}
