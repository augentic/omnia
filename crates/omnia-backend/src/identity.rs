//! Identity and authentication service contract, value types, and default implementations.

/// An `OAuth` 2.0 access token along with its expiration time.
///
/// Mirrors the `access-token` record from `omnia:identity/credentials`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AccessToken {
    /// The `OAuth` 2.0 access token.
    pub token: String,
    /// Seconds until this token expires.
    pub expires_in: u64,
}

#[cfg(not(target_arch = "wasm32"))]
mod host {
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
}

#[cfg(all(not(target_arch = "wasm32"), feature = "oauth"))]
mod oauth {
    use std::collections::HashMap;
    use std::fmt;
    use std::sync::Arc;
    use std::time::{Duration, Instant};

    use anyhow::{Context, Result};
    use futures::FutureExt as _;
    use futures::lock::Mutex;
    use oauth2::basic::{BasicClient, BasicTokenType};
    use oauth2::reqwest::{self, redirect};
    use oauth2::{
        ClientId, ClientSecret, EmptyExtraTokenFields, Scope, StandardTokenResponse,
        TokenResponse as _, TokenUrl,
    };
    use tracing::instrument;

    use super::{AccessToken, Identity, IdentityClient};
    use crate::{Connect, FutureResult};

    type TokenResponse = StandardTokenResponse<EmptyExtraTokenFields, BasicTokenType>;

    #[expect(missing_docs, reason = "`FromEnv` has no docs")]
    mod config {
        use fromenv::FromEnv;

        /// Connection options for [`IdentityDefault`](super::IdentityDefault).
        #[derive(Clone, FromEnv)]
        pub struct ConnectOptions {
            /// `OAuth2` client ID (`IDENTITY_CLIENT_ID`).
            #[env(from = "IDENTITY_CLIENT_ID")]
            pub client_id: String,
            /// `OAuth2` client secret (`IDENTITY_CLIENT_SECRET`).
            #[env(from = "IDENTITY_CLIENT_SECRET")]
            pub client_secret: String,
            /// `OAuth2` token endpoint URL (`IDENTITY_TOKEN_URL`).
            #[env(from = "IDENTITY_TOKEN_URL")]
            pub token_url: String,
        }
    }
    pub use config::ConnectOptions;

    impl fmt::Debug for ConnectOptions {
        fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
            f.debug_struct("ConnectOptions")
                .field("client_id", &self.client_id)
                .field("client_secret", &"<redacted>")
                .field("token_url", &self.token_url)
                .finish()
        }
    }

    impl crate::FromEnv for ConnectOptions {
        fn load_env() -> Result<Self> {
            Self::from_env().finalize().context("issue loading connection options")
        }
    }

    /// Default `OAuth2` client-credentials implementation of [`IdentityClient`].
    #[derive(Debug, Clone)]
    pub struct IdentityDefault {
        token_manager: TokenManager,
    }

    impl Connect for IdentityDefault {
        type ConnectOptions = ConnectOptions;

        #[instrument]
        async fn connect_with(options: Self::ConnectOptions) -> Result<Self> {
            let token_manager = TokenManager::new(options);
            Ok(Self { token_manager })
        }
    }

    impl IdentityClient for IdentityDefault {
        fn get_identity(&self, _name: String) -> FutureResult<Arc<dyn Identity>> {
            tracing::debug!("getting identity");
            let token_manager = self.token_manager.clone();
            async move { Ok(Arc::new(token_manager) as Arc<dyn Identity>) }.boxed()
        }
    }

    impl From<TokenResponse> for AccessToken {
        fn from(token_resp: TokenResponse) -> Self {
            let token = token_resp.access_token().secret().clone();
            let expires_in = token_resp.expires_in().unwrap_or(Duration::from_hours(1));

            Self {
                token,
                expires_in: expires_in.as_secs(),
            }
        }
    }

    #[derive(Debug, Clone)]
    struct CachedToken {
        access_token: AccessToken,
        expires_at: Instant,
    }

    impl CachedToken {
        fn new(access_token: AccessToken) -> Self {
            let ttl = Duration::from_secs(access_token.expires_in);
            let expires_at = Instant::now() + ttl;

            Self {
                access_token,
                expires_at,
            }
        }
    }

    #[derive(Debug, Clone)]
    struct TokenManager {
        options: Arc<ConnectOptions>,
        cache: Arc<Mutex<HashMap<Vec<String>, CachedToken>>>,
    }

    impl Identity for TokenManager {
        fn get_token(&self, scopes: Vec<String>) -> FutureResult<AccessToken> {
            tracing::debug!("getting token");
            let token_manager = self.clone();
            async move { token_manager.token(&scopes).await }.boxed()
        }
    }

    impl TokenManager {
        fn new(options: ConnectOptions) -> Self {
            Self {
                options: Arc::new(options),
                cache: Arc::new(Mutex::new(HashMap::new())),
            }
        }

        async fn token(&self, scopes: &[String]) -> Result<AccessToken> {
            let key = cache_key(scopes);
            let now = Instant::now();

            {
                let cache = self.cache.lock().await;
                if let Some(cached) = cache.get(&key)
                    && cached.expires_at > now
                {
                    return Ok(cached.access_token.clone());
                }
            }

            let oauth2_client = BasicClient::new(ClientId::new(self.options.client_id.clone()))
                .set_client_secret(ClientSecret::new(self.options.client_secret.clone()))
                .set_token_uri(TokenUrl::new(self.options.token_url.clone())?);
            let http_client =
                reqwest::ClientBuilder::new().redirect(redirect::Policy::none()).build()?;

            let mut token_req = oauth2_client.exchange_client_credentials();
            for scope in scopes {
                token_req = token_req.add_scope(Scope::new(scope.clone()));
            }

            let token_resp = token_req.request_async(&http_client).await?;
            let access_token = AccessToken::from(token_resp);

            let mut cache = self.cache.lock().await;
            let entry = cache.entry(key).or_insert_with(|| CachedToken::new(access_token.clone()));
            if entry.expires_at <= now {
                *entry = CachedToken::new(access_token);
            }
            let token = entry.access_token.clone();
            drop(cache);

            Ok(token)
        }
    }

    fn cache_key(scopes: &[String]) -> Vec<String> {
        let mut key = scopes.to_vec();
        key.sort_unstable();
        key.dedup();
        key
    }
}

#[cfg(not(target_arch = "wasm32"))]
pub use self::host::{Identity, IdentityClient, IdentityStub};
#[cfg(all(not(target_arch = "wasm32"), feature = "oauth"))]
pub use self::oauth::{ConnectOptions, IdentityDefault};
