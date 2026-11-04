//! Credential-free stub implementation of [`IdentityClient`].

use std::sync::Arc;

use anyhow::Result;
use futures::FutureExt as _;
use omnia_backend::identity::{AccessToken, Identity, IdentityClient};
use omnia_core::{Connect, FutureResult, NoOptions};

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
