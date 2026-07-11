use std::sync::Arc;

use anyhow::Result;
use futures::FutureExt;
use omnia::Backend;

use crate::host::WasiIdentityCtx;
use crate::host::generated::omnia::identity::credentials::AccessToken;
use crate::host::resource::{FutureResult, Identity};

/// Connection options for the stub (none required).
#[derive(Debug, Clone)]
pub struct StubOptions;

impl omnia::FromEnv for StubOptions {
    fn from_env() -> Result<Self> {
        Ok(Self)
    }
}

/// Credential-free `wasi:identity` backend returning a fixed token.
///
/// For tests and local development where no identity provider is available;
/// production deployments use [`super::IdentityDefault`] or a backend crate.
#[derive(Debug, Clone, Default)]
pub struct IdentityStub;

impl Backend for IdentityStub {
    type ConnectOptions = StubOptions;

    async fn connect_with(_options: Self::ConnectOptions) -> Result<Self> {
        Ok(Self)
    }
}

impl WasiIdentityCtx for IdentityStub {
    fn get_identity(&self, _name: String) -> FutureResult<Arc<dyn Identity>> {
        async { Ok(Arc::new(StubIdentity) as Arc<dyn Identity>) }.boxed()
    }
}

/// The fixed-token identity handed out by [`IdentityStub`].
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
