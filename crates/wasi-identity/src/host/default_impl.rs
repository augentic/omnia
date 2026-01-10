use std::sync::Arc;
use std::time::{Duration, Instant};

use anyhow::{Context, Result};
use fromenv::FromEnv;
use futures::FutureExt;
use futures::lock::Mutex;
use oauth2::basic::{BasicClient, BasicTokenType};
use oauth2::reqwest::{self, redirect};
use oauth2::{
    ClientId, ClientSecret, EmptyExtraTokenFields, Scope, StandardTokenResponse,
    TokenResponse as _, TokenUrl,
};
use tracing::instrument;
use yetti::Backend;

use crate::host::WasiIdentityCtx;
pub use crate::host::generated::wasi::identity::credentials::AccessToken;
use crate::host::resource::{FutureResult, Identity};

type TokenResponse = StandardTokenResponse<EmptyExtraTokenFields, BasicTokenType>;

#[derive(Debug, Clone, FromEnv)]
pub struct ConnectOptions {
    #[env(from = "IDENTITY_CLIENT_ID")]
    pub client_id: String,
    #[env(from = "IDENTITY_CLIENT_SECRET")]
    pub client_secret: String,
    #[env(from = "IDENTITY_TOKEN_URL")]
    pub token_url: String,
}

impl yetti::FromEnv for ConnectOptions {
    fn from_env() -> Result<Self> {
        Self::from_env().finalize().context("issue loading connection options")
    }
}

#[derive(Debug, Clone)]
pub struct IdentityDefault {
    token_manager: TokenManager,
}

impl Backend for IdentityDefault {
    type ConnectOptions = ConnectOptions;

    #[instrument]
    async fn connect_with(options: Self::ConnectOptions) -> Result<Self> {
        let token_manager = TokenManager::new(options);
        Ok(Self { token_manager })
    }
}

impl WasiIdentityCtx for IdentityDefault {
    fn get_identity(&self, _name: String) -> FutureResult<Arc<dyn Identity>> {
        tracing::debug!("getting identity");
        let token_manager = self.token_manager.clone();
        async move { Ok(Arc::new(token_manager) as Arc<dyn Identity>) }.boxed()
    }
}

#[allow(clippy::derivable_impls)]
impl Default for AccessToken {
    fn default() -> Self {
        Self {
            token: String::new(),
            expires_in: 0,
        }
    }
}

impl From<TokenResponse> for AccessToken {
    fn from(token_resp: TokenResponse) -> Self {
        let token = token_resp.access_token().secret().clone();
        let expires_in = token_resp.expires_in().unwrap_or(Duration::from_secs(3600));

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
    // TODO: change to use wasi-keyvalue for distributed caching
    cache: Arc<Mutex<CachedToken>>,
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
            cache: Arc::new(Mutex::new(CachedToken {
                access_token: AccessToken::default(),
                expires_at: Instant::now(),
            })),
        }
    }

    async fn token(&self, scopes: &[String]) -> Result<AccessToken> {
        let now = Instant::now();

        // use cached token if still valid
        {
            let cache = self.cache.lock().await;
            if cache.expires_at > now {
                return Ok(cache.access_token.clone());
            }
        }

        // if we drop through we need to fetch a new token
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

        // double-check locking as another thread may have refreshed the token
        let mut cache = self.cache.lock().await;
        if cache.expires_at <= now {
            *cache = CachedToken::new(access_token.clone());
        }

        Ok(cache.access_token.clone())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn uses_cached_token() {
        let manager = TokenManager::new(ConnectOptions {
            client_id: "test-client".to_string(),
            client_secret: "test-secret".to_string(),
            token_url: "https://example.com/token".to_string(),
        });

        // seed cache
        {
            let mut cache = manager.cache.lock().await;
            cache.access_token = AccessToken {
                token: "cached-token".to_string(),
                expires_in: 60,
            };
            cache.expires_at = Instant::now() + Duration::from_secs(60);
        };

        let token = manager.token(&[]).await.expect("token from cache");
        assert_eq!(token.token, "cached-token");
        assert!(token.expires_in > 0);
    }
}
