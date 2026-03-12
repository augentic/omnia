# Config Capability Example

**Demonstrates:** `Config` capability trait

## Overview

The `Config` trait provides access to environment variables and configuration values. It is the most commonly used capability -- virtually every handler needs at least one config value for URLs, database names, topic names, or identity references.

**Trait definition:** See [../../references/capabilities.md](../../references/capabilities.md#config)

## Simple Config Usage

A handler that reads configuration values to build a greeting:

```rust
use anyhow::Context as _;
use omnia_sdk::{Config, Context, Error, Handler, Reply, Result};
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct GreetingRequest {
    pub message: String,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct GreetingResponse {
    pub respondent: String,
    pub reply: String,
}

async fn greeting<P: Config>(
    _owner: &str,
    provider: &P,
    req: GreetingRequest,
) -> Result<GreetingResponse> {
    // Read a string config value
    let name = Config::get(provider, "RESPONDENT_NAME").await?;

    Ok(GreetingResponse {
        respondent: name,
        reply: req.message,
    })
}

impl<P: Config> Handler<P> for GreetingRequest {
    type Error = Error;
    type Input = Vec<u8>;
    type Output = GreetingResponse;

    async fn handle(self, ctx: Context<'_, P>) -> Result<Reply<GreetingResponse>> {
        Ok(greeting(ctx.owner, ctx.provider, self).await?.into())
    }

    fn from_input(input: Self::Input) -> Result<Self> {
        serde_json::from_slice(&input)
            .context("deserializing GreetingRequest")
            .map_err(Into::into)
    }
}
```

## Parsing Config Values

Config always returns `String`. Parse numeric or boolean values explicitly:

```rust
async fn process_with_timeout<P: Config + HttpRequest>(
    provider: &P,
    url: &str,
) -> Result<Response<Bytes>> {
    // Parse a numeric config value
    let timeout_secs: u64 = Config::get(provider, "TIMEOUT_SECS")
        .await?
        .parse()
        .context("parsing TIMEOUT_SECS as u64")?;

    // Parse a boolean config value
    let verbose: bool = Config::get(provider, "VERBOSE_LOGGING")
        .await
        .unwrap_or_else(|_| "false".to_string())
        .parse()
        .unwrap_or(false);

    if verbose {
        tracing::info!("fetching {url} with timeout {timeout_secs}s");
    }

    // Use config to build URL
    let base_url = Config::get(provider, "API_URL").await?;
    let full_url = format!("{base_url}{url}");

    let request = http::Request::builder()
        .method("GET")
        .uri(&full_url)
        .body(http_body_util::Empty::<Bytes>::new())?;

    HttpRequest::fetch(provider, request).await
}
```

## Conditional Logic Based on Config

Use config values to drive branching behavior:

```rust
const OUTPUT_TOPIC: &str = "events-output.v1";

async fn route_event<P: Config + Publish>(
    provider: &P,
    event: &Event,
) -> Result<()> {
    // Read env prefix and build topic name
    let env = Config::get(provider, "ENV").await.unwrap_or_else(|_| "dev".to_string());
    let topic = format!("{env}-{OUTPUT_TOPIC}");

    let payload = serde_json::to_vec(event)?;
    let message = Message::new(&payload);
    Publish::send(provider, &topic, &message).await?;

    Ok(())
}
```

## Config Keys in .env.example

Every config key used in the generated crate must appear in `.env.example`:

```bash
# .env.example
ENV=dev
RESPONDENT_NAME=example-name
API_URL=https://api.example.com
TIMEOUT_SECS=30
VERBOSE_LOGGING=false
DATABASE_NAME=main-db
AZURE_IDENTITY=my-identity
```

## Key Patterns

1. **Always use `Config::get(provider, "KEY")`** -- never `std::env::var("KEY")`
2. **Parse explicitly** -- Config returns `String`, parse to the required type
3. **Document all keys** -- Every key must appear in `.env.example`
4. **Use descriptive key names** -- `API_URL`, not `URL`; `DATABASE_NAME`, not `DB`
5. **Config is always in bounds** -- Include `Config` in every handler's provider bounds

## References

- See [../../references/capabilities.md](../../references/capabilities.md) for the full trait definition
- See [../../references/sdk-api.md](../../references/sdk-api.md) for the Handler trait pattern
- See [../../references/providers.md](../../references/providers.md) for provider bound composition
