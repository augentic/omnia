//! Identity/token capability.

use std::future::Future;

use anyhow::Result;

/// Interacts with identity providers to obtain access tokens.
pub trait Identity: Send + Sync {
    /// Get an access token for the specified identity.
    #[cfg(not(target_arch = "wasm32"))]
    fn access_token(&self, identity: String) -> impl Future<Output = Result<String>> + Send;

    /// Get an access token for the specified identity.
    #[cfg(target_arch = "wasm32")]
    fn access_token(&self, identity: String) -> impl Future<Output = Result<String>> + Send {
        use omnia_wasi_identity::credentials::get_identity;

        async move {
            let identity = get_identity(identity).await?;
            let access_token = identity.get_token(vec![]).await?;
            Ok(access_token.token)
        }
    }
}
