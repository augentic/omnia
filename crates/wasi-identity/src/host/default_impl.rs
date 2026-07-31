use std::collections::HashMap;
use std::fmt;
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
use omnia::Backend;
use tracing::instrument;

use crate::host::WasiIdentityCtx;
pub use crate::host::generated::omnia::identity::credentials::AccessToken;
use crate::host::resource::{FutureResult, Identity};

type TokenResponse = StandardTokenResponse<EmptyExtraTokenFields, BasicTokenType>;

#[derive(Clone, FromEnv)]
pub struct ConnectOptions {
    #[env(from = "IDENTITY_CLIENT_ID")]
    pub client_id: String,
    #[env(from = "IDENTITY_CLIENT_SECRET")]
    pub client_secret: String,
    #[env(from = "IDENTITY_TOKEN_URL")]
    pub token_url: String,
}

// Manual impl so the client secret can never leak through `Debug`-recording
// sinks (`#[instrument]` records arguments via `Debug` by default).
impl fmt::Debug for ConnectOptions {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ConnectOptions")
            .field("client_id", &self.client_id)
            .field("client_secret", &"<redacted>")
            .field("token_url", &self.token_url)
            .finish()
    }
}

impl omnia::FromEnv for ConnectOptions {
    fn load_env() -> Result<Self> {
        // `Self::from_env()` is the builder-returning inherent the `FromEnv`
        // derive emits.
        Self::from_env().finalize().context("issue loading connection options")
    }
}

/// Default implementation for `wasi:identity`.
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
    // Tokens are scoped: a token minted for one scope set must never be
    // handed out for another, so the cache is keyed by normalized scopes.
    // TODO: change to use wasi-keyvalue for distributed caching
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

        // use cached token if still valid
        {
            let cache = self.cache.lock().await;
            if let Some(cached) = cache.get(&key)
                && cached.expires_at > now
            {
                return Ok(cached.access_token.clone());
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

        // double-check locking as another task may have refreshed this entry
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

/// Normalize a scope list into a cache key (order- and duplicate-insensitive).
fn cache_key(scopes: &[String]) -> Vec<String> {
    let mut key = scopes.to_vec();
    key.sort_unstable();
    key.dedup();
    key
}
