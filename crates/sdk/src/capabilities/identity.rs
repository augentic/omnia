//! Identity and access-token capability.

use std::future::Future;

use crate::capability::Result;
use crate::identity::AccessToken;
#[cfg(all(target_arch = "wasm32", target_os = "wasi"))]
use crate::identity::{DEFAULT, Handle};

/// Obtain access tokens for the identities a deployment declares.
///
/// A handle acts as one identity. A provider's own handle uses
/// [`identity::DEFAULT`](crate::identity::DEFAULT), and
/// [`named`](Self::named) selects another. An identity the deployment does
/// not declare fails when a token is requested. The bare token string is
/// [`IdentityExt`].
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

#[cfg(all(target_arch = "wasm32", target_os = "wasi"))]
impl Identity for crate::WasiProvider {
    type Named = Handle;

    async fn token(&self, scopes: &[&str]) -> Result<AccessToken> {
        Handle::new(DEFAULT).token(scopes).await
    }

    fn named(&self, name: &str) -> Handle {
        Handle::new(name)
    }
}
