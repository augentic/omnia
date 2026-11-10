use std::sync::Arc;

use anyhow::Result;
use futures::FutureExt;
use omnia_core::Connect;

use crate::host::IdentityClient;
use crate::host::generated::omnia::identity::credentials::AccessToken;
use crate::host::resource::{FutureResult, Identity};

/// Credential-free `wasi:identity` backend returning a fixed token.
///
/// For tests and local development where no identity provider is available;
/// production deployments use `IdentityDefault` (the `oauth` feature) or a
/// backend crate.
#[derive(Debug, Clone, Default)]
pub struct IdentityStub;

impl Connect for IdentityStub {
    type ConnectOptions = omnia_core::NoOptions;

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
