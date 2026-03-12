# Anti-patterns

Contrastive examples showing frequent LLM code-generation mistakes and their correct alternatives. Each pair shows a **wrong** pattern and the **right** pattern.

## 1. Using `reqwest` instead of `HttpRequest` provider trait

WASM guests cannot use host-side HTTP clients. All HTTP calls must go through the Omnia `HttpRequest` provider trait.

**Wrong:**

```rust
use reqwest::Client;

async fn fetch_data(url: &str) -> Result<String> {
    let client = Client::new();
    let response = client.get(url).send().await?;
    Ok(response.text().await?)
}
```

**Right:**

```rust
use omnia_sdk::{HttpRequest, Result};

async fn fetch_data<P: HttpRequest>(provider: &P, url: &str) -> Result<Vec<u8>> {
    let request = http::Request::builder()
        .method("GET")
        .uri(url)
        .body(http_body_util::Empty::<bytes::Bytes>::new())?;

    let response = HttpRequest::fetch(provider, request).await?;
    Ok(response.into_body().to_vec())
}
```

**Why:** `reqwest` is a forbidden crate — it depends on `tokio`, `hyper`, and native TLS, none of which compile to wasm32. The `HttpRequest` provider trait routes the call through the WASI host.

## 2. Using `std::env::var` instead of `Config::get`

WASM guests have no access to environment variables. Configuration is injected by the host through the `Config` provider trait.

**Wrong:**

```rust
fn get_api_url() -> String {
    std::env::var("API_URL").expect("API_URL must be set")
}
```

**Right:**

```rust
use omnia_sdk::{Config, Result};

async fn get_api_url<P: Config>(provider: &P) -> Result<String> {
    Config::get(provider, "API_URL").await
}
```

**Why:** `std::env` is not available in wasm32. Even if it compiled, environment variables are not how WASI components receive configuration. The `Config` trait provides the host-managed configuration store.

## 3. Using typed `Input` instead of `Vec<u8>` with deserialization

The Omnia runtime delivers raw bytes to handlers. The `from_input` method must deserialize from `Vec<u8>`.

**Wrong:**

```rust
impl<P: Config> Handler<P> for MyRequest {
    type Input = MyRequest;  // Typed input — bypasses deserialization
    type Output = MyResponse;
    type Error = omnia_sdk::Error;

    fn from_input(input: Self::Input) -> Result<Self> {
        Ok(input)  // Identity — no actual parsing
    }
}
```

**Right:**

```rust
use anyhow::Context as _;

impl<P: Config> Handler<P> for MyRequest {
    type Input = Vec<u8>;  // Raw bytes from runtime
    type Output = MyResponse;
    type Error = omnia_sdk::Error;

    fn from_input(input: Self::Input) -> Result<Self> {
        serde_json::from_slice(&input)
            .context("deserializing MyRequest")
            .map_err(Into::into)
    }
}
```

**Why:** The Omnia runtime dispatches raw byte payloads to handlers. Using a typed `Input` means the handler cannot be invoked by the runtime. Always deserialize from `Vec<u8>` in `from_input`.

**Exception:** Scheduled/cron/health-check handlers that receive no payload use `type Input = ()`.

## 4. Missing `Identity` in handler bounds when auth is needed

When any HTTP call requires an authentication token, the handler must include `Identity` in its provider bounds and follow the Config → Identity → HttpRequest sequence.

**Wrong:**

```rust
impl<P: Config + HttpRequest> Handler<P> for AuthenticatedRequest {
    async fn handle(self, ctx: Context<'_, P>) -> Result<Reply<Self::Output>> {
        // No way to get a token — Identity is not in bounds!
        let request = http::Request::builder()
            .header("Authorization", "Bearer ???")
            .body(http_body_util::Empty::<bytes::Bytes>::new())?;

        let response = HttpRequest::fetch(ctx.provider, request).await?;
        // ...
    }
}
```

**Right:**

```rust
impl<P: Config + HttpRequest + Identity> Handler<P> for AuthenticatedRequest {
    async fn handle(self, ctx: Context<'_, P>) -> Result<Reply<Self::Output>> {
        let identity = Config::get(ctx.provider, "AZURE_IDENTITY").await?;
        let token = Identity::access_token(ctx.provider, identity).await?;

        let request = http::Request::builder()
            .header("Authorization", format!("Bearer {token}"))
            .body(http_body_util::Empty::<bytes::Bytes>::new())?;

        let response = HttpRequest::fetch(ctx.provider, request).await?;
        // ...
    }
}
```

**Why:** Without `Identity` in the trait bounds, there is no way to obtain an access token. The Config → Identity → HttpRequest sequence is the only supported auth flow in Omnia.

## 5. Using `static` / `OnceCell` for caching instead of `StateStore`

WASM components are stateless. Any caching or state persistence must go through the `StateStore` provider trait.

**Wrong:**

```rust
use std::sync::OnceLock;

static CACHE: OnceLock<HashMap<String, String>> = OnceLock::new();

fn get_cached(key: &str) -> Option<&String> {
    CACHE.get().and_then(|c| c.get(key))
}
```

**Right:**

```rust
use omnia_sdk::{StateStore, Result};

async fn get_cached<P: StateStore>(provider: &P, key: &str) -> Result<Option<Vec<u8>>> {
    StateStore::get(provider, key).await
}

async fn set_cached<P: StateStore>(
    provider: &P,
    key: &str,
    value: &[u8],
    ttl_secs: Option<u64>,
) -> Result<Option<Vec<u8>>> {
    StateStore::set(provider, key, value, ttl_secs).await
}
```

**Why:** WASM components are instantiated fresh for each invocation. Global statics are not shared across invocations and violate the statelessness requirement. `StateStore` provides host-managed persistent caching.

## 6. Using bidirectional `serde(rename)` on input-only types

Input types (e.g., XML messages) should use deserialize-only renames so that if the struct is ever serialized (for logging, caching, StateStore), it uses the Rust field name rather than the foreign field name.

**Wrong:**

```rust
// Bidirectional rename — serializes back to Spanish XML field names
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct TrainUpdate {
    #[serde(rename = "trenPar")]
    pub even_train_id: String,

    #[serde(rename = "trenImpar")]
    pub odd_train_id: String,

    #[serde(rename = "fechaCreacion")]
    pub created_date: String,
}
```

**Right:**

```rust
// Deserialize-only rename — serializes with Rust field names
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
pub struct TrainUpdate {
    #[serde(rename(deserialize = "trenPar"))]
    pub even_train_id: Option<String>,

    #[serde(rename(deserialize = "trenImpar"))]
    pub odd_train_id: Option<String>,

    #[serde(rename(deserialize = "fechaCreacion"))]
    #[serde(deserialize_with = "r9k_date")]
    pub created_date: NaiveDate,
}
```

**Why:** Bidirectional `rename` causes the struct to serialize with foreign field names (e.g., Spanish XML names), which is confusing in logs and unexpected if the struct is cached via StateStore. Use `rename(deserialize = "...")` for input-only types so serialization uses the Rust field name.

## 7. Missing message key on published messages

When the artifacts document a partition key or message key for published messages, it must be set on the `Message` headers. Missing keys affect Kafka consumer ordering guarantees.

**Wrong:**

```rust
let payload = serde_json::to_vec(&event)?;
let message = Message::new(&payload);
// No key set — messages won't be partitioned correctly
Publish::send(provider, &topic, &message).await?;
```

**Right:**

```rust
let payload = serde_json::to_vec(&event)?;
let external_id = &event.remote_data.external_id;

let mut message = Message::new(&payload);
message.headers.insert("key".to_string(), external_id.clone());

Publish::send(provider, &topic, &message).await?;
```

**Why:** Without a partition key, Kafka distributes messages across partitions arbitrarily. Downstream consumers that depend on ordering (e.g., processing all events for a vehicle in order) will see interleaved messages. Always set the key when the artifacts document one.

## 8. Temporal validation in `from_input()`

Validation that compares against current time must NOT be in `from_input()` because it runs at parse time, not invocation time.

**Wrong:**

```rust
fn from_input(input: Self::Input) -> Result<Self> {
    let msg: Self = quick_xml::de::from_reader(input.as_ref())
        .context("parsing XML")
        .map_err(Into::into)?;

    // WRONG: Uses Utc::now() — will compute delay at PARSE time
    let delay_secs = compute_delay(&msg.created_date)?;
    if delay_secs > MAX_DELAY_SECS {
        return Err(R9kError::BadTime("outdated".into()).into());
    }

    Ok(msg)
}
```

**Right:**

```rust
fn from_input(input: Self::Input) -> Result<Self> {
    quick_xml::de::from_reader(input.as_ref())
        .context("parsing XML")
        .map_err(Into::into)
    // NO time-based validation here
}

impl R9kMessage {
    fn validate(&self) -> Result<()> {
        // Temporal validation HERE — runs at invocation time
        let delay_secs = compute_delay(&self.created_date)?;
        if delay_secs > MAX_DELAY_SECS {
            return Err(R9kError::BadTime(format!("outdated by {delay_secs}s")).into());
        }
        Ok(())
    }
}

async fn handle<P>(_owner: &str, request: R9kMessage, provider: &P) -> Result<Reply<()>>
where
    P: Config + HttpRequest + Publish,
{
    request.validate()?; // Time validation here
    // ... business logic ...
}
```

**Why:** Message replay and test fixtures cannot shift time in `from_input()`. Validation using `Utc::now()` must run in `handle()`.

## 9. Handler Input Type Confusion

The Omnia runtime delivers raw bytes. Using typed `Input` bypasses deserialization and causes type errors.

**Wrong:**

```rust
impl<P: Config> Handler<P> for R9kMessage {
    type Input = R9kMessage;  // Typed input
    type Output = ();
    type Error = Error;

    fn from_input(input: Self::Input) -> Result<Self> {
        Ok(input)  // Identity function — no parsing
    }
}
```

**Errors you'll see:**

```text
the trait bound `Vec<u8>: omnia_sdk::Handler<MockProvider>` is not satisfied
the trait `omnia_sdk::Handler<MockProvider>` is not implemented for `Vec<u8>`
```

**Right:**

```rust
impl<P: Config> Handler<P> for R9kMessage {
    type Input = Vec<u8>;  // Raw bytes from runtime
    type Output = ();
    type Error = Error;

    fn from_input(input: Self::Input) -> Result<Self> {
        quick_xml::de::from_reader(input.as_ref())
            .context("deserializing R9K message")
            .map_err(Into::into)
    }
}
```

**Why:** The Omnia runtime dispatches raw byte payloads to handlers. Using a typed `Input` means the handler cannot be invoked by the runtime. Always deserialize from `Vec<u8>` in `from_input`.

**Exception:** Scheduled/cron/health-check handlers use `type Input = ()` because they receive no payload.

## 10. Using `HttpRequest` for Azure Table Storage instead of `TableStore`

When the source code or artifacts describe access to Azure Table Storage (via `@azure/data-tables`, REST API calls, `TableClient`, etc.), use `TableStore` — not `HttpRequest`. The Omnia runtime provides a native Azure Table Storage adapter behind `TableStore`.

**Wrong:**

```rust
use omnia_sdk::{Config, HttpRequest, Result};

async fn fetch_fleet_data<P>(provider: &P) -> Result<Vec<RawVehicle>>
where
    P: Config + HttpRequest,
{
    let storage_account = Config::get(provider, "STORAGE_ACC").await?;
    let storage_key = Config::get(provider, "STORAGE_KEY").await?;
    let url = format!(
        "https://{storage_account}.table.core.windows.net/fleetdata()"
    );

    let request = http::Request::builder()
        .method("GET")
        .uri(&url)
        .header("Accept", "application/json;odata=nometadata")
        .header("x-ms-version", "2019-02-02")
        .header("Authorization", format!("SharedKey {storage_account}:{storage_key}"))
        .body(http_body_util::Empty::<bytes::Bytes>::new())?;

    let response = HttpRequest::fetch(provider, request).await?;
    let wrapper: AzureTableResponse = serde_json::from_slice(&response.into_body())?;
    Ok(wrapper.value)
}
```

**Right:**

```rust
use omnia_sdk::{Config, Result};
use omnia_wasi_sql::entity;
use omnia_wasi_sql::orm::{SelectBuilder, TableStore};

entity! {
    table = "fleetdata",
    #[derive(Clone, Debug, Deserialize, Serialize)]
    pub struct RawVehicle {
        pub partition_key: String,
        pub row_key: String,
        pub call_sign: Option<String>,
        pub label: Option<String>,
        // ... remaining Azure Table Storage entity fields
    }
}

async fn fetch_fleet_data<P>(provider: &P) -> Result<Vec<RawVehicle>>
where
    P: Config + TableStore,
{
    let db_name = Config::get(provider, "FLEET_TABLE_STORE").await?;
    let vehicles = SelectBuilder::<RawVehicle>::new()
        .fetch(provider, &db_name)
        .await
        .map_err(|e| bad_gateway!("failed to fetch fleet data: {e}"))?;
    Ok(vehicles)
}
```

**Why:** The Omnia runtime provides a native adapter for Azure Table Storage behind the `TableStore` trait. Constructing raw HTTP requests with SharedKey authentication headers is unnecessary, error-prone (HMAC-SHA256 signature generation is complex), and bypasses the runtime's connection management and authentication handling. The `entity!` macro and ORM builders work with Azure Table Storage entities the same way they work with SQL rows — Azure Table Storage being NoSQL is irrelevant to trait selection.
