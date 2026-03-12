# Provider Patterns

Provider trait composition, guest-side Provider struct configuration, and trait-by-trait reference for production and test use.

For trait definitions and method signatures, see [capabilities.md](../capabilities.md).

---

## Trait Summary

| Trait           | When Required                                       | Key Points                                        | Reference                          |
| --------------- | --------------------------------------------------- | ------------------------------------------------- | ---------------------------------- |
| **Config**      | Always -- all components read environment variables | Return all config keys; error on unknown keys     | [config.md](config.md)             |
| **HttpRequest** | Component makes HTTP calls                          | URI pattern matching; realistic responses         | [http-request.md](http-request.md) |
| **Publish**     | Component publishes messages/events                 | Capture messages; provide deserialization helpers | [publish.md](publish.md)           |
| **Identity**    | ANY HTTP call uses Bearer authentication            | Return realistic tokens; track requests           | [identity.md](identity.md)         |
| **StateStore**  | Component uses caching or key-value storage         | OnceCell + Mutex; handle TTL                      | [state-store.md](state-store.md)   |
| **Broadcast**   | Handler sends data to WebSocket clients             | Capture sends; verify channel and targets         | [broadcast.md](broadcast.md)       |

---

## Provider Configuration

### Overview

The Provider struct implements WASI capability traits, bridging domain logic to WASI implementations. Domain crates declare which traits they need; the guest's Provider struct satisfies those bounds with empty implementations that delegate to the Omnia SDK defaults.

### Owner

Every handler invocation requires an `owner` parameter -- a hardcoded string identifying the Omnia component owner (e.g. `"at"`). This is specified in the builder chain:

```rust
MyRequest::handler(input)?
    .provider(&Provider::new())
    .owner("at")        // <-- required owner identifier
    .await
    .map_err(Into::into)
```

When using the `guest!` macro, `owner` is declared once at the top level:

```rust
omnia_sdk::guest!({
    owner: "at",
    provider: Provider,
    // ...
});
```

The owner value is determined by the organization or tenant that owns the Omnia deployment. It is typically a short string (e.g. `"at"`) and must be consistent across all handlers in a guest.

### Available Traits

See the [Capability Selection Summary](../capabilities.md#capability-selection-summary) for the full list of traits and their Specify triggers. For trait definitions and method signatures, see [capabilities.md](../capabilities.md).

### Marker Provider (Simple)

Use when default SDK behavior is sufficient. Only implement the traits that domain crates actually require.

#### With Config Validation

Use `ensure_env!` when the guest requires environment variables at startup:

```rust
use omnia_sdk::{Broadcast, Config, HttpRequest, Identity, Publish, StateStore, ensure_env};

#[derive(Clone, Default)]
pub struct Provider;

impl Provider {
    #[must_use]
    pub fn new() -> Self {
        ensure_env!(
            "BLOCK_MGT_URL",
            "FLEET_URL",
            "GTFS_STATIC_URL",
        );
        Self
    }
}

// Empty implementations use SDK defaults
impl Broadcast for Provider {}
impl Config for Provider {}
impl HttpRequest for Provider {}
impl Identity for Provider {}
impl Publish for Provider {}
impl StateStore for Provider {}
```

#### Without Config Validation

When no environment variables are needed, Provider can be a `const fn`:

```rust
use omnia_sdk::{Config, HttpRequest, StateStore};

#[derive(Clone, Default)]
pub struct Provider;

impl Provider {
    #[must_use]
    pub const fn new() -> Self {
        Self
    }
}

impl Config for Provider {}
impl HttpRequest for Provider {}
impl StateStore for Provider {}
```

### ensure_env! Macro (Optional)

Validates required environment variables at startup. Use only when the guest requires config values:

```rust
ensure_env!(
    "API_URL",
    "SERVICE_NAME",
    "AZURE_IDENTITY",
);
```

Fails fast with clear error messages if any variable is missing. If the guest has no config requirements, omit `ensure_env!` entirely and make `Provider::new()` a `const fn`.

---

## Provider Trait Composition

Domain crates declare provider trait bounds on their functions and handler implementations. They never implement providers, construct host-side types, or call raw WASI modules directly. All external I/O flows through the generic `provider: &P` parameter.

### Provider Bounds on Functions

Each function accepts a generic `provider: &P` with **only the traits it needs**:

```rust
pub async fn fetch_and_enrich<P>(provider: &P, id: &str) -> Result<Enriched>
where
    P: Config + HttpRequest,
{
    let api_url = Config::get(provider, "API_URL").await?;
    let request = http::Request::builder()
        .method("GET")
        .uri(format!("{api_url}/items/{id}"))
        .body(Empty::<Bytes>::new())?;
    let response = HttpRequest::fetch(provider, request).await?;
    // ...
}
```

### Minimal Composition Rule

Include **only** the traits the function actually calls. If a function only reads config and makes HTTP calls, its bound is `P: Config + HttpRequest` -- not `P: Config + HttpRequest + Publish + StateStore + Identity`.

This keeps the function testable (fewer mock traits) and self-documenting (bounds declare exactly what I/O occurs).

### Provider Bounds on Handlers

The `Handler<P>` implementation declares the full set of traits needed by the handler and all functions it calls:

```rust
impl<P> Handler<P> for MyRequest
where
    P: Config + HttpRequest + Publish,
{
    type Input = Vec<u8>;
    type Output = MyResponse;
    type Error = omnia_sdk::Error;

    fn from_input(input: Self::Input) -> Result<Self> {
        serde_json::from_slice(&input)
            .context("deserializing MyRequest")
            .map_err(Into::into)
    }

    async fn handle(self, ctx: Context<'_, P>) -> Result<Reply<Self::Output>> {
        let result = process_logic(ctx.provider, &self).await?;
        Ok(result.into())
    }
}
```

The handler's trait bounds are the **union** of all traits required by the internal functions it calls.

### Composing Trait Bounds

When a handler calls multiple functions with different bounds, the handler's bound is the union:

```rust
// Function A needs Config + HttpRequest
async fn fetch_data<P: Config + HttpRequest>(provider: &P, id: &str) -> Result<Data> { ... }

// Function B needs Config + Publish
async fn publish_event<P: Config + Publish>(provider: &P, event: &Event) -> Result<()> { ... }

// Handler needs Config + HttpRequest + Publish (the union)
impl<P: Config + HttpRequest + Publish> Handler<P> for MyRequest {
    async fn handle(self, ctx: Context<'_, P>) -> Result<Reply<MyResponse>> {
        let data = fetch_data(ctx.provider, &self.id).await?;
        publish_event(ctx.provider, &data.event).await?;
        Ok(MyResponse { data }.into())
    }
}
```

### The Authentication Pattern

When any HTTP call requires a Bearer token, the handler must include `Identity` in its bounds and follow this sequence:

```rust
// 1. Read identity name from config
let identity = Config::get(provider, "AZURE_IDENTITY").await?;

// 2. Fetch access token
let token = Identity::access_token(provider, identity).await?;

// 3. Attach token to HTTP request
let request = http::Request::builder()
    .header("Authorization", format!("Bearer {token}"))
    // ...
```

This means the handler bounds become `P: Config + Identity + HttpRequest` at minimum.

See [identity.md](identity.md) for mock implementations of the Identity trait.

### Rules

1. **Never construct host-side types** -- No `Client::new()`, `RedisClient::connect()`, `Producer::new()`, etc.
2. **Never create I/O abstractions** -- Don't wrap provider traits in custom abstractions.
3. **Never call raw WASI modules** -- Domain crates use only `omnia_sdk` traits. Raw WASI calls (`omnia_wasi_http::handle`, etc.) belong in boundary/provider code.
4. **All state is explicit** -- No caching, memoization, or global state. All state flows through function parameters and provider calls.
5. **Config, not env vars** -- Use `Config::get(provider, "KEY")`, never `std::env::var("KEY")`.

### Selecting Traits from Artifacts

See [capabilities.md](../capabilities.md#capability-selection-summary) for the full artifact-to-capability mapping table.

For both artifact types, check the "Source Capabilities Summary" section and "External Service Dependencies". Use `crate-writer/references/capability-mapping.md` to map generic capabilities to Omnia traits.

For code-analysis artifacts, also derive traits from "Business Logic Blocks".

---

## Multi-Trait MockProvider

When a component uses multiple traits, combine the individual implementations. All test provider files should use `#![allow(missing_docs)]` at the top of the file since test code does not need doc comments.

### Complete Example

```rust
use omnia_sdk::{Config, HttpRequest, Publish, Identity, StateStore, Message};
use std::sync::{Arc, Mutex};
use std::collections::HashMap;
use once_cell::sync::OnceCell;

static CACHE: OnceCell<Mutex<HashMap<String, Vec<u8>>>> = OnceCell::new();

fn cache() -> &'static Mutex<HashMap<String, Vec<u8>>> {
    CACHE.get_or_init(|| Mutex::new(HashMap::new()))
}

#[derive(Clone)]
pub struct MockProvider {
    published: Arc<Mutex<Vec<Message>>>,
}

impl MockProvider {
    pub fn new() -> Self {
        cache();
        Self {
            published: Arc::new(Mutex::new(Vec::new())),
        }
    }

    pub fn published_messages(&self) -> Vec<Message> {
        self.published.lock().unwrap().clone()
    }

    pub fn cache_contains(&self, key: &str) -> bool {
        cache().lock().unwrap().contains_key(key)
    }
}

impl Config for MockProvider {
    async fn get(&self, key: &str) -> anyhow::Result<String> {
        match key {
            "API_URL" => Ok("https://example.com".to_string()),
            "CACHE_BUCKET" => Ok("test-bucket".to_string()),
            "TOPIC" => Ok("test-topic".to_string()),
            _ => anyhow::bail!("unknown config key: {key}"),
        }
    }
}

impl HttpRequest for MockProvider {
    async fn fetch<T>(
        &self, request: http::Request<T>,
    ) -> anyhow::Result<http::Response<bytes::Bytes>>
    where
        T: http_body::Body + std::any::Any + Send,
        T::Data: Into<Vec<u8>>,
        T::Error: Into<Box<dyn std::error::Error + Send + Sync + 'static>>,
    {
        let uri = request.uri().to_string();

        if uri.contains("/api/data") {
            let body = serde_json::json!({"result": "success"});
            Ok(http::Response::builder()
                .status(200)
                .body(bytes::Bytes::from(serde_json::to_vec(&body)?))?)
        } else {
            Ok(http::Response::builder()
                .status(404)
                .body(bytes::Bytes::from("Not Found"))?)
        }
    }
}

impl Publish for MockProvider {
    async fn send(&self, _topic: &str, message: &Message) -> anyhow::Result<()> {
        self.published.lock().unwrap().push(message.clone());
        Ok(())
    }
}

impl Identity for MockProvider {
    async fn access_token(&self, _identity: String) -> anyhow::Result<String> {
        Ok("mock_access_token".to_string())
    }
}

impl StateStore for MockProvider {
    async fn get(&self, key: &str) -> anyhow::Result<Option<Vec<u8>>> {
        let cache = cache().lock().unwrap();
        Ok(cache.get(key).cloned())
    }

    async fn set(
        &self,
        key: &str,
        value: &[u8],
        _ttl_secs: Option<u64>,
    ) -> anyhow::Result<Option<Vec<u8>>> {
        let previous = cache()
            .lock()
            .unwrap()
            .insert(key.to_string(), value.to_vec());
        Ok(previous)
    }

    async fn delete(&self, key: &str) -> anyhow::Result<()> {
        cache().lock().unwrap().remove(key);
        Ok(())
    }
}
```

## References

- [capabilities.md](../capabilities.md) -- Trait definitions and method signatures
- [guest-patterns.md](../guest-patterns.md) -- Guest export patterns (HTTP, Messaging, WebSocket)
- [runtime.md](../runtime.md) -- Local development runtime setup
