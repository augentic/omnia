use std::collections::BTreeSet;
use std::env::{self, VarError};
use std::sync::Arc;

use anyhow::{Context, Result};
use futures::FutureExt;
use omnia_core::Connect;

use crate::host::generated::omnia::identity::credentials::AccessToken;
use crate::host::resource::{FutureResult, Identity};
use crate::host::{Error, IdentityClient};

const DEFAULT: &str = "default";

const TOKEN: &str = "stub-token";

/// The identities an [`IdentityStub`] serves.
///
/// [`FromEnv`](omnia_core::FromEnv) reads them from `IDENTITY_NAMES`, a
/// comma-separated list that is `default` alone when unset.
#[derive(Clone, Debug)]
pub struct StubOptions {
    /// The names of the identities served.
    pub names: Vec<String>,
}

impl Default for StubOptions {
    fn default() -> Self {
        Self {
            names: vec![DEFAULT.to_owned()],
        }
    }
}

impl omnia_core::FromEnv for StubOptions {
    fn load_env() -> Result<Self> {
        let names = match env::var("IDENTITY_NAMES") {
            Ok(names) => names,
            Err(VarError::NotPresent) => return Ok(Self::default()),
            Err(err) => return Err(err).context("reading `IDENTITY_NAMES`"),
        };
        let names = names.split(',').map(str::trim).filter(|name| !name.is_empty());
        Ok(Self {
            names: names.map(str::to_owned).collect(),
        })
    }
}

/// Credential-free `wasi:identity` backend issuing a fixed token per identity.
///
/// For tests and local development where no identity provider is available;
/// production deployments use `IdentityDefault` (the `oauth` feature) or a
/// backend crate.
///
/// It serves the identities its [`StubOptions`] name and refuses every other
/// name. The `default` identity's token is `stub-token`, and any other
/// identity's is `stub-token:<name>`, so each identity's token is distinct.
/// Every token lasts an hour.
#[derive(Debug, Clone)]
pub struct IdentityStub {
    names: Arc<BTreeSet<String>>,
}

impl Default for IdentityStub {
    fn default() -> Self {
        Self::new(StubOptions::default())
    }
}

impl IdentityStub {
    fn new(options: StubOptions) -> Self {
        Self {
            names: Arc::new(options.names.into_iter().collect()),
        }
    }
}

impl Connect for IdentityStub {
    type ConnectOptions = StubOptions;

    fn connect_with(
        options: Self::ConnectOptions,
    ) -> impl std::future::Future<Output = Result<Self>> {
        std::future::ready(Ok(Self::new(options)))
    }
}

impl IdentityClient for IdentityStub {
    fn get_identity(&self, name: String) -> FutureResult<Arc<dyn Identity>> {
        let served = self.names.contains(&name);
        async move {
            anyhow::ensure!(served, Error::NoSuchIdentity);
            let token = if name == DEFAULT { TOKEN.to_owned() } else { format!("{TOKEN}:{name}") };
            Ok(Arc::new(StubIdentity { token }) as Arc<dyn Identity>)
        }
        .boxed()
    }
}

#[derive(Debug)]
struct StubIdentity {
    token: String,
}

impl Identity for StubIdentity {
    fn get_token(&self, _scopes: Vec<String>) -> FutureResult<AccessToken> {
        let token = self.token.clone();
        async move {
            Ok(AccessToken {
                token,
                expires_in: 3600,
            })
        }
        .boxed()
    }
}
