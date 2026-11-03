//! Service contract and credential-free stub for `omnia:identity` backends.

use std::fmt::Debug;
use std::sync::Arc;

use anyhow::Result;
use futures::FutureExt as _;

use super::AccessToken;
use crate::{Connect, FutureResult, NoOptions};

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

/// Credential-free [`IdentityClient`] returning a fixed token.
#[derive(Debug, Clone, Default)]
pub struct IdentityStub;

impl Connect for IdentityStub {
    type ConnectOptions = NoOptions;

    fn connect_with(
        _options: Self::ConnectOptions,
    ) -> impl std::future::Future<Output = Result<Self>> {
        std::future::ready(Ok(Self))
    }
}

impl IdentityClient for IdentityStub {
    fn get_identity(&self, _name: String) -> FutureResult<Arc<dyn Identity>> {
        async { Ok(Arc::new(StubIdentity) as Arc<dyn Identity>) }.boxed()
    }
}

#[derive(Debug)]
struct StubIdentity;

impl Identity for StubIdentity {
    fn get_token(&self, _scopes: Vec<String>) -> FutureResult<AccessToken> {
        async {
            Ok(AccessToken {
                token: "stub-token".to_string(),
                expires_in: 3600,
            })
        }
        .boxed()
    }
}
