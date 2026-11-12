//! Identity/token capability.

use std::future::Future;

use anyhow::Result;

/// An `OAuth` 2.0 access token along with its expiration time.
///
/// Mirrors the `access-token` record from `omnia:identity/credentials`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AccessToken {
    /// The `OAuth` 2.0 access token.
    pub token: String,
    /// Seconds until this token expires.
    pub expires_in: u64,
}

/// Interacts with identity providers to obtain access tokens.
///
/// The bare token string under default scopes is [`IdentityExt`].
#[omnia_guest_macros::capability]
pub trait Identity: Send + Sync {
    /// Obtain an access token for `identity` scoped to `scopes`.
    fn token(
        &self, identity: &str, scopes: &[String],
    ) -> impl Future<Output = Result<AccessToken>> + Send;
}

/// Token shorthands over any [`Identity`].
///
/// Blanket-implemented for every [`Identity`]; never implemented by hand.
pub trait IdentityExt: Identity {
    /// Get an access token string for `identity` with default scopes.
    fn access_token(&self, identity: &str) -> impl Future<Output = Result<String>> + Send {
        async move { Ok(self.token(identity, &[]).await?.token) }
    }
}

impl<P: Identity + ?Sized> IdentityExt for P {}

#[cfg(all(target_arch = "wasm32", target_os = "wasi"))]
impl Identity for crate::WasiProvider {
    async fn token(&self, identity: &str, scopes: &[String]) -> Result<AccessToken> {
        use omnia_wasi_identity::credentials::get_identity;

        let identity = get_identity(identity.to_owned()).await?;
        let access_token = identity.get_token(scopes.to_vec()).await?;
        Ok(AccessToken {
            token: access_token.token,
            expires_in: access_token.expires_in,
        })
    }
}
