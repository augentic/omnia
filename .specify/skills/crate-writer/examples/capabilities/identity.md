# Identity Capability Example

**Demonstrates:** `Identity` capability trait (typically used with `Config` and `HttpRequest`)

## Overview

The `Identity` trait fetches access tokens from identity providers (e.g., Azure AD). It is used whenever an outbound HTTP call requires authentication. The standard pattern is Config -> Identity -> HttpRequest: read the identity name from config, fetch a token, then attach it as a Bearer header.

**Trait definition:** See [../../references/capabilities.md](../../references/capabilities.md#identity)

## Basic Authentication Flow

A handler that fetches data from an authenticated API:

```rust
use anyhow::Context as _;
use bytes::Bytes;
use http_body_util::Empty;
use omnia_sdk::{
    Config, Context, Error, Handler, HttpRequest, Identity, Reply, Result,
    bad_gateway, server_error,
};
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct UserProfileRequest {
    pub user_id: String,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct UserProfileResponse {
    pub display_name: String,
    pub email: String,
}

async fn fetch_user_profile<P>(
    _owner: &str,
    provider: &P,
    req: UserProfileRequest,
) -> Result<UserProfileResponse>
where
    P: Config + Identity + HttpRequest,
{
    // Step 1: Read identity name from config
    let identity = Config::get(provider, "AZURE_IDENTITY").await?;

    // Step 2: Fetch access token
    let token = Identity::access_token(provider, identity).await?;

    // Step 3: Build authenticated HTTP request
    let api_url = Config::get(provider, "GRAPH_API_URL").await?;
    let url = format!("{api_url}/users/{}", req.user_id);

    let request = http::Request::builder()
        .method("GET")
        .uri(&url)
        .header("Authorization", format!("Bearer {token}"))
        .header("Accept", "application/json")
        .body(Empty::<Bytes>::new())
        .map_err(|err| server_error!("failed to build HTTP request: {err}"))?;

    // Step 4: Execute request
    let response = HttpRequest::fetch(provider, request)
        .await
        .map_err(|err| bad_gateway!("API request failed: {err}"))?;

    // Step 5: Parse response
    let profile: UserProfileResponse = serde_json::from_slice(response.body())
        .map_err(|err| server_error!("failed to parse profile response: {err}"))?;

    Ok(profile)
}

impl<P: Config + Identity + HttpRequest> Handler<P> for UserProfileRequest {
    type Error = Error;
    type Input = Vec<u8>;
    type Output = UserProfileResponse;

    async fn handle(self, ctx: Context<'_, P>) -> Result<Reply<UserProfileResponse>> {
        Ok(fetch_user_profile(ctx.owner, ctx.provider, self).await?.into())
    }

    fn from_input(input: Self::Input) -> Result<Self> {
        serde_json::from_slice(&input)
            .context("deserializing UserProfileRequest")
            .map_err(Into::into)
    }
}
```

## Authenticated POST with Body

Sending data to an authenticated API:

```rust
async fn submit_report<P>(provider: &P, report: &Report) -> Result<SubmitResult>
where
    P: Config + Identity + HttpRequest,
{
    // Authenticate
    let identity = Config::get(provider, "AZURE_IDENTITY").await?;
    let token = Identity::access_token(provider, identity).await?;

    // Build request with JSON body
    let api_url = Config::get(provider, "REPORTS_API_URL").await?;
    let body = serde_json::to_vec(report)?;

    let request = http::Request::builder()
        .method("POST")
        .uri(&api_url)
        .header("Authorization", format!("Bearer {token}"))
        .header("Content-Type", "application/json")
        .body(body.into())
        .map_err(|err| server_error!("failed to build request: {err}"))?;

    let response = HttpRequest::fetch(provider, request)
        .await
        .map_err(|err| bad_gateway!("submit failed: {err}"))?;

    serde_json::from_slice(response.body())
        .map_err(|err| server_error!("failed to parse response: {err}"))
}
```

## Multiple Authenticated Calls

When a handler makes multiple authenticated calls, fetch the token once and reuse it:

```rust
async fn sync_data<P>(provider: &P, ids: &[String]) -> Result<Vec<SyncResult>>
where
    P: Config + Identity + HttpRequest,
{
    // Fetch token once
    let identity = Config::get(provider, "AZURE_IDENTITY").await?;
    let token = Identity::access_token(provider, identity).await?;
    let api_url = Config::get(provider, "DATA_API_URL").await?;

    let mut results = Vec::with_capacity(ids.len());

    for id in ids {
        let request = http::Request::builder()
            .method("GET")
            .uri(format!("{api_url}/items/{id}"))
            .header("Authorization", format!("Bearer {token}"))
            .header("Accept", "application/json")
            .body(Empty::<Bytes>::new())?;

        let response = HttpRequest::fetch(provider, request).await?;
        let item: SyncResult = serde_json::from_slice(response.body())?;
        results.push(item);
    }

    Ok(results)
}
```

## Identity + Publish Pattern

Combining authentication with event publishing:

```rust
const OUTPUT_TOPIC: &str = "events-output.v1";

async fn fetch_and_publish<P>(provider: &P, event_id: &str) -> Result<()>
where
    P: Config + Identity + HttpRequest + Publish,
{
    // Authenticated fetch
    let identity = Config::get(provider, "AZURE_IDENTITY").await?;
    let token = Identity::access_token(provider, identity).await?;
    let api_url = Config::get(provider, "API_URL").await?;

    let request = http::Request::builder()
        .method("GET")
        .uri(format!("{api_url}/events/{event_id}"))
        .header("Authorization", format!("Bearer {token}"))
        .body(Empty::<Bytes>::new())?;

    let response = HttpRequest::fetch(provider, request).await?;

    // Publish result
    let env = Config::get(provider, "ENV").await.unwrap_or_else(|_| "dev".to_string());
    let topic = format!("{env}-{OUTPUT_TOPIC}");
    let message = Message::new(response.body());
    Publish::send(provider, &topic, &message).await?;

    Ok(())
}
```

## Key Patterns

1. **Config -> Identity -> HttpRequest** -- always follow this sequence for authenticated calls
2. **Read identity name from Config** -- never hardcode identity strings; use `Config::get(provider, "AZURE_IDENTITY")`
3. **Include Identity in bounds** -- if ANY HTTP call requires auth, the handler must have `P: Config + Identity + HttpRequest`
4. **Reuse tokens within a handler** -- call `Identity::access_token` once and reuse the token for multiple requests within the same handler invocation
5. **Bearer format** -- always use `format!("Bearer {token}")` in the `Authorization` header

```bash
# .env.example
ENV=dev
AZURE_IDENTITY=my-identity
GRAPH_API_URL=https://graph.microsoft.com/v1.0
REPORTS_API_URL=https://api.example.com/reports
DATA_API_URL=https://api.example.com/data
API_URL=https://api.example.com
```

## References

- See [../../references/capabilities.md](../../references/capabilities.md) for the full `Identity` trait definition
- See [http-request.md](http-request.md) for basic HTTP patterns
- See [publisher.md](publisher.md) for the publishing pattern
- See [../../references/sdk-api.md](../../references/sdk-api.md) for the Handler trait pattern
- See [../../references/providers.md](../../references/providers.md) for provider bound composition
