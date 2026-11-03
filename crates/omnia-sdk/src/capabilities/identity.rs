//! Identity/token capability.

use std::future::Future;

use anyhow::Result;
pub use omnia_backend::identity::AccessToken;

/// Interacts with identity providers to obtain access tokens.
#[omnia_guest_macros::capability]
pub trait Identity: Send + Sync {
    /// Obtain an access token for `identity` scoped to `scopes`.
    fn token(
        &self, identity: &str, scopes: &[String],
    ) -> impl Future<Output = Result<AccessToken>> + Send;

    /// Get an access token string for the specified identity with default scopes.
    fn access_token(&self, identity: String) -> impl Future<Output = Result<String>> + Send {
        async move { Ok(self.token(&identity, &[]).await?.token) }
    }
}

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
