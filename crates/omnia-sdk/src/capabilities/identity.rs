//! Identity/token capability.

use std::future::Future;

use anyhow::Result;
pub use omnia_backend::identity::AccessToken;

/// Interacts with identity providers to obtain access tokens.
pub trait Identity: Send + Sync {
    cfg_select! {
        all(target_arch = "wasm32", target_os = "wasi") => {
            /// Obtain an access token for `identity` scoped to `scopes`.
            fn token(
                &self, identity: &str, scopes: &[String],
            ) -> impl Future<Output = Result<AccessToken>> + Send {
                use omnia_wasi_identity::credentials::get_identity;

                let identity = identity.to_owned();
                let scopes = scopes.to_vec();
                async move {
                    let identity = get_identity(identity).await?;
                    let access_token = identity.get_token(scopes).await?;
                    Ok(AccessToken {
                        token: access_token.token,
                        expires_in: access_token.expires_in,
                    })
                }
            }
        }
        _ => {
            /// Obtain an access token for `identity` scoped to `scopes`.
            fn token(
                &self, identity: &str, scopes: &[String],
            ) -> impl Future<Output = Result<AccessToken>> + Send;
        }
    }

    /// Get an access token string for the specified identity with default scopes.
    fn access_token(&self, identity: String) -> impl Future<Output = Result<String>> + Send {
        async move { Ok(self.token(&identity, &[]).await?.token) }
    }
}

delegate_deref!(Identity {
    fn token(
        &self, identity: &str, scopes: &[String],
    ) -> impl Future<Output = Result<AccessToken>> + Send {
        (**self).token(identity, scopes)
    }

    fn access_token(&self, identity: String) -> impl Future<Output = Result<String>> + Send {
        (**self).access_token(identity)
    }
});
