# Example HTTP Handlers

This document combines the example HTTP handler implementation from `crates/ex-http/src/` in the augentic/context repository.

**Demonstrates:** `HttpRequest` and `Config` capability traits

## lib.rs

```rust
//! Handlers and provider for the HTTP example.
//!
//! This crate is defined separately to the core example so it can be tested.
//! Tests cannot run under the `wasm32-wasip2` target, so this allows us to
//! use configuration flags for this target in the main example crate.
mod handlers;

pub use handlers::*;
```

## handlers.rs

```rust
//! HTTP request handlers demonstrating the handler pattern.
//!
//! Handlers are domain-layer business logic that:
//! - Are WASM-agnostic (can run in native or WASM)
//! - Depend on provider traits, not concrete implementations
//! - Use strongly typed request/response types
//! - Implement the Handler<P> trait for uniform invocation

use anyhow::Context as _;
use percent_encoding::percent_decode_str;
use omnia_sdk::{Config, Context, Error, Handler, Reply, Result, bad_request};
use serde::{Deserialize, Serialize};

/// Example of a strongly typed request, expected to be serialized as query
/// parameters of an HTTP GET request.
///
/// Axum extracts this from URL query string: `/?a=hello&b=world`
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct EchoRequest {
    pub a: String,
    pub b: String,
}

/// Response from a handler for an `EchoRequest`.
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct EchoResponse {
    pub a: String,
    pub b: String,
}

/// An example handler that takes a strongly typed request and returns a
/// strongly typed response.
///
/// This handler demonstrates:
/// - URL percent-decoding of query parameters
/// - Error handling with descriptive messages
/// - Minimal provider dependencies (no Config needed)
///
/// # Errors
/// * Returns `bad_request` if URL decoding fails for query parameters.
#[allow(clippy::unused_async)]
async fn echo(_owner: &str, _provider: &impl Config, req: EchoRequest) -> Result<EchoResponse> {
    let EchoRequest { a, b } = req;

    // Helper to decode percent-encoded query parameters
    let decode = |value: String, field: &str| -> Result<String> {
        percent_decode_str(&value)
            .decode_utf8()
            .map(std::borrow::Cow::into_owned)
            .map_err(|err| bad_request!("failed to decode '{field}': {err}"))
    };

    Ok(EchoResponse { a: decode(a, "a")?, b: decode(b, "b")? })
}

/// Common handler implementation for a consistent API.
///
/// This trait implementation allows the handler to be invoked uniformly via
/// `Client::request()` regardless of the specific request/response types.
impl<P: Config> Handler<P> for EchoRequest {
    type Error = Error;
    type Input = Vec<u8>;
    type Output = EchoResponse;

    async fn handle(self, ctx: Context<'_, P>) -> Result<Reply<EchoResponse>> {
        Ok(echo(ctx.owner, ctx.provider, self).await?.into())
    }

    fn from_input(input: Self::Input) -> Result<Self> {
        serde_json::from_slice(&input).context("deserializing EchoRequest").map_err(Into::into)
    }
}

/// Example of a strongly typed request, expected to be serialized as the body
/// of an HTTP request.
///
/// Axum extracts this from JSON request body via `Json<GreetingRequest>`.
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct GreetingRequest {
    pub message: String,
}

/// Example of a strongly typed response, expected to be serialized as the body
/// of an HTTP response.
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct GreetingResponse {
    /// Name of the respondent (fetched from configuration)
    pub respondent: String,
    /// Echo of the original message
    pub reply: String,
}

/// An example handler that takes a strongly typed request and returns a
/// strongly typed response.
///
/// This handler demonstrates:
/// - Using the Config provider to fetch configuration values
/// - Composing response from both config and request data
///
/// There is a dependency on a provider that implements the `Config` trait for
/// configuration information.
///
/// # Errors
/// * The provider fails to retrieve the configuration value.
async fn greeting<P>(_owner: &str, provider: &P, req: GreetingRequest) -> Result<GreetingResponse>
where
    P: Config,
{
    // Fetch the respondent name from configuration
    let name = Config::get(provider, "name").await?;
    Ok(GreetingResponse { respondent: name, reply: req.message })
}

/// Common handler implementation for a consistent API.
impl<P: Config> Handler<P> for GreetingRequest {
    type Error = Error;
    type Input = Vec<u8>;
    type Output = GreetingResponse;

    async fn handle(self, ctx: Context<'_, P>) -> Result<Reply<GreetingResponse>> {
        Ok(greeting(ctx.owner, ctx.provider, self).await?.into())
    }

    fn from_input(input: Self::Input) -> Result<Self> {
        serde_json::from_slice(&input).context("deserializing GreetingRequest").map_err(Into::into)
    }
}
```

## Key Patterns Demonstrated

1. **Strongly Typed Requests/Responses**: Both `EchoRequest` and `GreetingRequest` are concrete types
2. **Handler Trait Implementation**: Both implement `Handler<P>` for uniform invocation
3. **Provider Dependencies**: Handlers depend on provider traits (e.g., `Config`)
4. **Error Handling**: Using `anyhow::Context` and `omnia_sdk::Error`
5. **Separation of Concerns**: Business logic is in separate functions (`echo`, `greeting`)
6. **WASM-Compatible**: No OS-specific dependencies, all async

## References

- See [../../references/sdk-api.md](../../references/sdk-api.md) for the Handler trait pattern
- See [../../references/capabilities.md](../../references/capabilities.md) for trait definitions
- See [../../references/providers.md](../../references/providers.md) for provider bound composition
