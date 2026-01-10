use std::fmt::Debug;
use std::ops::Deref;
use std::sync::Arc;

pub use yetti::FutureResult;

pub use crate::host::generated::wasi::identity::credentials::AccessToken;

/// Providers implement the [`Identity`] trait to allow the host to
/// interact with different backend identity providers.
pub trait Identity: Debug + Send + Sync + 'static {
    /// The name of the identity provider.
    fn get_token(&self, scopes: Vec<String>) -> FutureResult<AccessToken>;
}

/// Represents an identity resource in the WASI Vault.
#[derive(Debug, Clone)]
pub struct IdentityProxy(pub Arc<dyn Identity>);

impl Deref for IdentityProxy {
    type Target = Arc<dyn Identity>;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}
