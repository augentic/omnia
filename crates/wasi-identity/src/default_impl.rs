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
use omnia_core::Connect;
use tracing::instrument;

use crate::resource::{AccessToken, FutureResult, Identity};
use crate::{Error, IdentityClient};

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

impl omnia_core::FromEnv for ConnectOptions {
    fn load_env() -> Result<Self> {
        // `Self::from_env()` is the builder the `FromEnv` derive emits
        Self::from_env().finalize().context("issue loading connection options")
    }
}

/// The `OAuth` 2.0 client-credentials backend for `omnia:identity`.
///
/// Its one set of client credentials is the identity `default`, and it
/// refuses every other name with [`Error::NotFound`].
///
/// - It caches a token per set of scopes, and reports the time a cached token
///   has left.
/// - An issuer that states no expiry is taken to issue tokens that last an
///   hour.
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
    fn get_identity(&self, name: String) -> FutureResult<Arc<dyn Identity>> {
        tracing::debug!("getting identity `{name}`");
        let token_manager = self.token_manager.clone();
        async move {
            anyhow::ensure!(
                name == "default",
                Error::NotFound(format!("identity `{name}` is not declared"))
            );
            Ok(Arc::new(token_manager) as Arc<dyn Identity>)
        }
        .boxed()
    }
}

#[derive(Debug, Clone)]
struct TokenManager {
    options: Arc<ConnectOptions>,
    // keyed by normalized scopes: a token minted for one scope set is never
    // handed out for another
    // TODO: change to use wasi-keyvalue for distributed caching
    cache: Arc<Mutex<HashMap<Vec<String>, CachedToken>>>,
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

        // use cached token if still valid
        {
            let cache = self.cache.lock().await;
            if let Some(token) = cache.get(&key).and_then(|cached| cached.at(Instant::now())) {
                return Ok(token);
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
        let now = Instant::now();

        // double-check locking as another task may have refreshed this entry
        let mut cache = self.cache.lock().await;
        if let Some(token) = cache.get(&key).and_then(|cached| cached.at(now)) {
            return Ok(token);
        }
        cache.insert(key, CachedToken::new(&access_token, now));
        drop(cache);

        Ok(access_token)
    }
}

impl Identity for TokenManager {
    fn get_token(&self, scopes: Vec<String>) -> FutureResult<AccessToken> {
        tracing::debug!("getting token");
        let token_manager = self.clone();
        async move { token_manager.token(&scopes).await }.boxed()
    }
}

struct CachedToken {
    token: String,
    issued_at: Instant,
    lifetime: Duration,
}

impl CachedToken {
    fn new(access_token: &AccessToken, issued_at: Instant) -> Self {
        Self {
            token: access_token.token.clone(),
            issued_at,
            lifetime: Duration::from_secs(access_token.expires_in),
        }
    }

    // the token and the whole seconds it has left at `now`, or `None` once
    // less than a second is left, since the seam reports nothing finer
    fn at(&self, now: Instant) -> Option<AccessToken> {
        let left = self.lifetime.saturating_sub(now.saturating_duration_since(self.issued_at));
        (left.as_secs() > 0).then(|| AccessToken {
            token: self.token.clone(),
            expires_in: left.as_secs(),
        })
    }
}

// order- and duplicate-insensitive
fn cache_key(scopes: &[String]) -> Vec<String> {
    let mut key = scopes.to_vec();
    key.sort_unstable();
    key.dedup();
    key
}

type TokenResponse = StandardTokenResponse<EmptyExtraTokenFields, BasicTokenType>;

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
