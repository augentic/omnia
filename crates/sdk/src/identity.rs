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
        Ok(AccessToken::new(token.token, time_left(token.expires_at)))
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

// the time left before `expires_at`, an instant in milliseconds since the Unix
// epoch, and zero once it has passed
fn time_left(expires_at: u64) -> Duration {
    UNIX_EPOCH.checked_add(Duration::from_millis(expires_at)).map_or(Duration::MAX, |expiry| {
        expiry.duration_since(SystemTime::now()).unwrap_or_default()
    })
}

/// An `OAuth` 2.0 access token and how long it stays valid.
///
/// Its [`Debug`] form redacts the token.
#[derive(Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct AccessToken {
    /// The `OAuth` 2.0 access token.
    pub token: String,

    /// How long the token stays valid, counted from when
    /// [`token`](Identity::token) returned it.
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
