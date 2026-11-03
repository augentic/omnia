//! Service contract for `omnia:identity` backends.

use std::fmt::Debug;
use std::sync::Arc;

use super::AccessToken;
use crate::FutureResult;

/// Service contract for `omnia:identity` backends.
pub trait IdentityClient: Debug + Send + Sync + 'static {
    /// Get the identity resource for the specified `name`.
    fn get_identity(&self, name: String) -> FutureResult<Arc<dyn Identity>>;
}

/// Stateful handle to an identity provider.
pub trait Identity: Debug + Send + Sync + 'static {
    /// Obtain an access token scoped to `scopes`.
    fn get_token(&self, scopes: Vec<String>) -> FutureResult<AccessToken>;
}
