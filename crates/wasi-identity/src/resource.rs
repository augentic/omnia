use std::fmt::{self, Debug};

pub use omnia_core::FutureResult;

/// Providers implement the [`Identity`] trait to allow the host to
/// interact with different backend identity providers.
pub trait Identity: Debug + Send + Sync + 'static {
    /// A token for this identity, scoped to `scopes`.
    ///
    /// An empty `scopes` asks for the identity's default scopes. A failure
    /// the backend returns as an [`Error`](crate::Error) reaches the guest as
    /// that case, and any other failure is `other`.
    fn get_token(&self, scopes: Vec<String>) -> FutureResult<AccessToken>;
}

/// A token a backend issues for an [`Identity`], and how long it stays valid.
///
/// Its [`Debug`] form redacts the token.
#[derive(Clone)]
pub struct AccessToken {
    /// The `OAuth` 2.0 bearer token.
    pub token: String,

    /// The whole seconds the token stays valid, counted from when the backend
    /// answers.
    pub expires_in: u64,
}

impl Debug for AccessToken {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("AccessToken")
            .field("token", &"<redacted>")
            .field("expires_in", &self.expires_in)
            .finish()
    }
}

/// Represents an identity resource in the WASI Identity host.
pub type IdentityProxy = omnia_core::Proxy<dyn Identity>;
