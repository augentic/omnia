//! Identity and access-token capability.

use std::fmt;
use std::future::Future;
use std::time::Duration;

use crate::capability::Result;

/// Obtain access tokens for the identities a deployment declares.
///
/// A handle acts as one identity. A provider's own handle uses
/// [`identity::DEFAULT`](DEFAULT), and [`named`](Self::named) selects
/// another. An identity the deployment does not declare fails with
/// [`ErrorKind::NotFound`](crate::capability::ErrorKind::NotFound) when a
/// token is requested. The bare token string is [`IdentityExt`].
#[omnia_guest_macros::capability]
pub trait Identity: Send + Sync {
    /// The handle [`named`](Self::named) returns.
    type Named: Identity<Named = Self::Named>;

    /// An access token for this identity, scoped to `scopes`.
    ///
    /// An empty `scopes` asks for the identity's default scopes.
    fn token(&self, scopes: &[&str]) -> impl Future<Output = Result<AccessToken>> + Send;

    /// A handle to the identity `name` on the same provider.
    fn named(&self, name: &str) -> Self::Named;
}

/// Token shorthands over any [`Identity`].
///
/// Blanket-implemented for every [`Identity`]; never implemented by hand.
pub trait IdentityExt: Identity {
    /// The token string for this identity's default scopes.
    fn access_token(&self) -> impl Future<Output = Result<String>> + Send {
        async move { Ok(self.token(&[]).await?.token) }
    }
}

impl<P: Identity + ?Sized> IdentityExt for P {}

/// The identity a provider's own [`Identity`] handle uses.
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
