# Provider Patterns

Provider struct configuration, trait composition rules, and MockProvider patterns for generated crates.

For trait definitions and method signatures, see [capabilities.md](capabilities.md).

---

## Guest-Side Provider Struct

The Provider struct implements WASI capability traits, bridging domain logic to WASI implementations. Domain crates declare which traits they need; the guest's Provider struct satisfies those bounds with empty implementations that delegate to the Omnia SDK defaults.

### With Config Validation

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

impl Broadcast for Provider {}
impl Config for Provider {}
impl HttpRequest for Provider {}
impl Identity for Provider {}
impl Publish for Provider {}
impl StateStore for Provider {}
```

### Without Config Validation

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

### ensure_env! Macro

Validates required environment variables at startup. Fails fast with clear error messages if any variable is missing. If the guest has no config requirements, omit `ensure_env!` entirely and make `Provider::new()` a `const fn`.

```rust
ensure_env!(
    "API_URL",
    "SERVICE_NAME",
    "AZURE_IDENTITY",
);
```

### Owner

Every handler invocation requires an `owner` parameter -- a hardcoded string identifying the Omnia component owner (e.g. `"at"`). When using the `guest!` macro, `owner` is declared once at the top level:

```rust
omnia_sdk::guest!({
    owner: "at",
    provider: Provider,
    // ...
});
```

---

## Provider Trait Composition

Domain crates declare provider trait bounds on their functions and handler implementations. They never implement providers, construct host-side types, or call raw WASI modules directly. All external I/O flows through the generic `provider: &P` parameter.

### Bounds on Functions

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

### Bounds on Handlers

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
async fn fetch_data<P: Config + HttpRequest>(provider: &P, id: &str) -> Result<Data> { ... }

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

### Rules

1. **Never construct host-side types** -- No `Client::new()`, `RedisClient::connect()`, `Producer::new()`, etc.
2. **Never create I/O abstractions** -- Don't wrap provider traits in custom abstractions.
3. **Never call raw WASI modules** -- Domain crates use only `omnia_sdk` traits. Raw WASI calls (`omnia_wasi_http::handle`, etc.) belong in boundary/provider code.
4. **All state is explicit** -- No caching, memoization, or global state. All state flows through function parameters and provider calls.
5. **Config, not env vars** -- Use `Config::get(provider, "KEY")`, never `std::env::var("KEY")`.

### Selecting Traits from Artifacts

For both artifact types, check the "Source Capabilities Summary" section and map each checked capability to an Omnia trait using [capability-mapping.md](capability-mapping.md). Cross-reference with "External Service Dependencies" and "Business Logic Blocks" to verify completeness.

See the [Capability Selection Summary](capabilities.md#capability-selection-summary) for the full artifact-to-capability mapping table.

---

## MockProvider (Test Code)

When a component uses multiple traits, combine the individual implementations. All test provider files should use `#![allow(missing_docs)]` at the top of the file since test code does not need doc comments.

For complete multi-trait MockProvider examples and per-trait implementation patterns, see the [canonical provider references](providers/README.md#multi-trait-mockprovider).

### MockProvider Best Practices

- **Config**: return all config keys used by component; error on unknown keys (catches typos)
- **HttpRequest**: match on URI patterns, not exact strings; return realistic response structures
- **Publish**: use `Arc<Mutex<Vec<Message>>>` for thread-safe capture; provide deserialization helpers
- **Identity**: return realistic token format; track token requests for verification
- **StateStore**: use `OnceCell` for global cache state; return previous value from `set()`
- **Broadcast**: capture sends with channel and target info for assertions

## References

- [capabilities.md](capabilities.md) -- Trait definitions and method signatures
- [guest-wiring.md](guest-wiring.md) -- Guest export patterns (HTTP, Messaging, WebSocket)
- [sdk-api.md](sdk-api.md) -- Handler, Context, Reply, Error API
