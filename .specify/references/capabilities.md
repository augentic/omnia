# Provider Capability Traits

All 7 traits from `omnia_sdk::capabilities`. Each trait has default WASM32 implementations that use WASI bindings -- the guest Provider struct needs only empty `impl` blocks.

**Source of truth:** [`omnia-sdk/src/capabilities.rs`](https://github.com/augentic/omnia/blob/main/crates/omnia-sdk/src/capabilities.rs)

## Overview

Generated domain crates run as WASI guests inside the Omnia runtime. They cannot access the OS directly (no `std::fs`, `std::net`, `std::env`, `std::thread`). Instead, all external I/O flows through **capability traits** defined in `omnia-sdk`. The runtime provides concrete implementations of these traits via the provider.

Domain crate code uses the traits as generic bounds on functions and handler implementations. The code never implements or constructs the provider -- it only declares which capabilities it needs.

## Config

Read runtime configuration values (environment variables, config store).

| Entity              | Name                                                         |
| ------------------- | ------------------------------------------------------------ |
| **Crate**           | `omnia_sdk`                                                  |
| **WASI module**     | `omnia_wasi_config`                                          |
| **Import**          | `use omnia_sdk::Config;`                                     |
| **Always required** | Yes -- virtually all handlers need at least one config value |

```rust
pub trait Config: Send + Sync {
    fn get(&self, key: &str) -> impl Future<Output = Result<String>> + Send;
}
```

**Usage** (from r9k-adapter):

```rust
let env = Config::get(provider, "ENV").await.unwrap_or_else(|_| "dev".to_string());
let url = Config::get(provider, "BLOCK_MGT_URL").await?;
```

**Include when**: handler needs environment variables, API URLs, feature flags.

**Specify triggers**: any environment variable or configuration value referenced in the artifacts. Always include `Config` in handler bounds as a baseline.

**Cargo.toml**: no extra dependencies needed.

## HttpRequest

Make outbound HTTP requests to external APIs.

|                 |                               |
| --------------- | ----------------------------- |
| **Crate**       | `omnia_sdk`                   |
| **WASI module** | `omnia_wasi_http`             |
| **Import**      | `use omnia_sdk::HttpRequest;` |

```rust
pub trait HttpRequest: Send + Sync {
    fn fetch<T>(&self, request: Request<T>) -> impl Future<Output = Result<Response<Bytes>>> + Send
    where
        T: Body + Any + Send,
        T::Data: Into<Vec<u8>>,
        T::Error: Into<Box<dyn Error + Send + Sync + 'static>>;
}
```

**Usage** (from r9k-adapter):

```rust
use bytes::Bytes;
use http_body_util::Empty;

let request = http::Request::builder()
    .uri(format!("{url}/allocations/trips?externalRefId={}", self.train_id()))
    .header(AUTHORIZATION, format!("Bearer {token}"))
    .body(Empty::<Bytes>::new())
    .context("building request")?;
let response = HttpRequest::fetch(provider, request)
    .await
    .context("fetching train allocations")?;

let bytes = response.into_body();
let data: Vec<String> = serde_json::from_slice(&bytes)
    .context("deserializing response")?;
```

**Usage** (from cars -- with API key header):

```rust
let api_key = Config::get(provider, "MWS_API_KEY").await?;

let request = http::Request::builder()
    .uri(format!("{MWS_URI}/worksite-search?filter={filter}"))
    .header("x-api-key", &api_key)
    .body(Empty::<Bytes>::new())
    .context("building request")?;
let response = HttpRequest::fetch(provider, request)
    .await
    .map_err(|e| bad_gateway!("fetching worksites: {e}"))?;
```

**Include when**: handler calls external HTTP APIs (third-party REST services, partner integrations, external microservices).

**Specify triggers**: external HTTP calls, REST API integrations; any `fetch`, `axios`, `got`, or HTTP client usage in code-analysis artifacts; any API endpoint calls in requirements artifacts.

**Exclusion — managed data stores**: Do NOT use `HttpRequest` when the artifacts or source code describe access to a managed storage service that has a dedicated Omnia trait. Even if the source code constructs raw REST API calls (e.g., `https://{account}.table.core.windows.net`), use the corresponding storage trait instead:

| Service | Correct Trait | NOT HttpRequest |
|---------|---------------|-----------------|
| Azure Table Storage | `TableStore` | Never raw HTTP to `*.table.core.windows.net` |
| Azure Cosmos DB | `TableStore` | Never raw HTTP to `*.documents.azure.com` |
| Redis / Memcached | `StateStore` | Never raw HTTP to cache endpoints |
| SQL databases | `TableStore` | Never raw HTTP to database endpoints |

The Omnia runtime provides native adapters for these services behind the respective traits. Constructing raw HTTP requests with SharedKey/HMAC/SAS authentication to storage service REST APIs is always incorrect.

**Cargo.toml**: requires `bytes`, `http`, `http-body`, `http-body-util`.

### HttpError

The SDK also exports `HttpError` for typed HTTP error handling:

```rust
use omnia_sdk::HttpError;
```

`HttpError` is returned when an `HttpRequest::fetch` call fails at the HTTP level (e.g., connection refused, timeout). It is distinct from application-level errors parsed from response bodies.

## Publish

Send messages to topics (Kafka, message broker).

|                 |                                      |
| --------------- | ------------------------------------ |
| **Crate**       | `omnia_sdk`                          |
| **WASI module** | `omnia_wasi_messaging`               |
| **Import**      | `use omnia_sdk::{Publish, Message};` |

```rust
#[derive(Clone, Debug)]
pub struct Message {
    pub payload: Vec<u8>,
    pub headers: HashMap<String, String>,
}

impl Message {
    pub fn new(payload: &[u8]) -> Self {
        Self {
            payload: payload.to_vec(),
            headers: HashMap::new(),
        }
    }
}

pub trait Publish: Send + Sync {
    fn send(&self, topic: &str, message: &Message) -> impl Future<Output = Result<()>> + Send;
}
```

**Usage** (from r9k-adapter):

```rust
use omnia_sdk::Message;

let payload = serde_json::to_vec(&event).context("serializing event")?;
let mut message = Message::new(&payload);
message.headers.insert("key".to_string(), external_id.clone());

Publish::send(provider, &topic, &message).await?;
```

**Topic naming pattern**:

```rust
const OUTPUT_TOPIC: &str = "realtime-r9k-to-smartrak.v1";

let env = Config::get(provider, "ENV").await.unwrap_or_else(|_| "dev".to_string());
let topic = format!("{env}-{OUTPUT_TOPIC}");
```

**Include when**: handler publishes messages to topics.

**Specify triggers**: event publishing, message sending, queue operations; any `producer.send`, `producer.sendBatch` in code-analysis artifacts; any messaging/event publishing in requirements artifacts.

**Cargo.toml**: no extra dependencies beyond `serde_json`.

## StateStore

Key-value store for caching (Redis-backed).

|                 |                              |
| --------------- | ---------------------------- |
| **Crate**       | `omnia_sdk`                  |
| **WASI module** | `omnia_wasi_keyvalue`        |
| **Import**      | `use omnia_sdk::StateStore;` |

```rust
pub trait StateStore: Send + Sync {
    /// Retrieve a previously stored value.
    fn get(&self, key: &str) -> impl Future<Output = Result<Option<Vec<u8>>>> + Send;

    /// Store a value. Returns the previous value if one existed.
    fn set(
        &self, key: &str, value: &[u8], ttl_secs: Option<u64>,
    ) -> impl Future<Output = Result<Option<Vec<u8>>>> + Send;

    /// Delete a value.
    fn delete(&self, key: &str) -> impl Future<Output = Result<()>> + Send;
}
```

**Usage**:

```rust
// Read from cache
let cached = StateStore::get(provider, "cache-key").await?;
if let Some(bytes) = cached {
    let data: MyData = serde_json::from_slice(&bytes)?;
    return Ok(data);
}

// Write to cache (with 5-minute TTL)
let bytes = serde_json::to_vec(&data)?;
StateStore::set(provider, "cache-key", &bytes, Some(300)).await?;

// Delete from cache
StateStore::delete(provider, "cache-key").await?;
```

**Include when**: handler needs caching or state persistence across invocations. Common methods using global singletons for managing state must be avoided (see [guardrails.md](guardrails.md)); use `StateStore` instead.

**Specify triggers**: state storage, caching, Redis operations; any `redis.get`, `redis.set`, `redis.del` in code-analysis artifacts; any state/cache requirements in requirements artifacts.

**Cargo.toml**: no extra dependencies.

**Important:** `set` returns `Result<Option<Vec<u8>>>` (the previous value), not `Result<()>`.

## Identity

Obtain access tokens from identity providers (Azure AD, etc.).

|                 |                            |
| --------------- | -------------------------- |
| **Crate**       | `omnia_sdk`                |
| **WASI module** | `omnia_wasi_identity`      |
| **Import**      | `use omnia_sdk::Identity;` |

```rust
pub trait Identity: Send + Sync {
    fn access_token(&self, identity: String) -> impl Future<Output = Result<String>> + Send;
}
```

**Usage** (from r9k-adapter -- Config -> Identity -> HttpRequest pattern):

```rust
let identity = Config::get(provider, "AZURE_IDENTITY").await?;
let token = Identity::access_token(provider, identity).await?;

let request = http::Request::builder()
    .uri(url)
    .header(AUTHORIZATION, format!("Bearer {token}"))
    .body(Empty::<Bytes>::new())?;
let response = HttpRequest::fetch(provider, request).await?;
```

**Include when**: any HTTP call requires authentication tokens. Always pair with `Config` (for the identity name) and `HttpRequest`.

**Specify triggers**: Azure AD token acquisition, OAuth flows; any authenticated HTTP calls; any `Identity` or token-based auth in requirements artifacts.

**Cargo.toml**: no extra dependencies.

## TableStore

Database and table storage access (queries, CRUD, and statements). Covers **both** relational SQL databases **and** managed NoSQL table stores (Azure Table Storage, Cosmos DB). An ORM layer is available via `omnia_orm`.

> **WARNING — Do not be misled by the `omnia_wasi_sql` module name.** The WASI module is named `omnia_wasi_sql` for historical reasons, but `TableStore` is a **general-purpose data access abstraction** used by the Omnia runtime for SQL databases, Azure Table Storage, Azure Cosmos DB, and other tabular/document stores. The runtime provides native adapters for each backend behind this single trait. When you see `@azure/data-tables`, `TableClient`, `listEntities`, or `*.table.core.windows.net` in source code, use `TableStore` — not `HttpRequest` or `StateStore`. Azure Table Storage being "NoSQL" or "not SQL" is **irrelevant** to trait selection.

|                 |                                                                                        |
| --------------- | -------------------------------------------------------------------------------------- |
| **Crate**       | `omnia_sdk`                                                                            |
| **WASI module** | `omnia_wasi_sql` (name is historical — covers both SQL and NoSQL stores)               |
| **Import**      | `use omnia_orm::{SelectBuilder, InsertBuilder, UpdateBuilder, DeleteBuilder, Filter};` |

```rust
use omnia_wasi_sql::{DataType, Row};

pub trait TableStore: Send + Sync {
    fn query(
        &self, cnn_name: String, query: String, params: Vec<DataType>,
    ) -> impl Future<Output = Result<Vec<Row>>> + Send;

    fn exec(
        &self, cnn_name: String, query: String, params: Vec<DataType>,
    ) -> impl Future<Output = Result<u32>> + Send;
}
```

**Usage (raw)**:

```rust
use omnia_wasi_sql::DataType;

let rows = TableStore::query(
    provider,
    "my-database".to_string(),
    "SELECT id, name FROM items WHERE status = ?".to_string(),
    vec![DataType::Str("active".to_string())],
).await?;

let affected = TableStore::exec(
    provider,
    "my-database".to_string(),
    "UPDATE items SET status = ? WHERE id = ?".to_string(),
    vec![DataType::Str("archived".to_string()), DataType::Int32(item_id)],
).await?;
```

**Usage (ORM -- preferred for CRUD)**:

```rust
use omnia_wasi_sql::entity;
use omnia_orm::{Filter, SelectBuilder, TableStore};

entity! {
    table = "users",
    #[derive(Clone, Debug, Deserialize, Serialize)]
    pub struct User {
        pub id: i32,
        pub name: String,
        pub email: String,
    }
}

let db_name = Config::get(provider, "DATABASE_NAME").await?;
let users = SelectBuilder::<User>::new()
    .r#where(Filter::eq("active", true))
    .limit(100)
    .fetch(provider, &db_name)
    .await?;
```

**Include when**: handler needs database or table storage access — SQL databases, Azure Table Storage, Azure Cosmos DB, or any managed data store. For Azure Table Storage, always include `PartitionKey` and `RowKey` fields in update and delete filter criteria, and always send an entire object for update.

**CRITICAL — Azure Table Storage is TableStore, NOT HttpRequest**: When the source code or artifacts describe access to Azure Table Storage (via `@azure/data-tables` SDK, REST API calls to `*.table.core.windows.net`, SharedKey/SAS authentication, or `TableClient.listEntities`), use `TableStore` — never `HttpRequest`. Azure Table Storage being "NoSQL" is irrelevant; the Omnia runtime provides a native Azure Table Storage adapter behind the `TableStore` trait. The `entity!` macro and ORM builders work with Azure Table Storage entities just as they do with SQL rows.

**Specify triggers**: SQL database operations, CRUD patterns; any database queries or table references in requirements artifacts; any SQL/ORM operations in code-analysis artifacts; **Azure Table Storage access** (including `@azure/data-tables`, `TableClient`, `listEntities`, `table.core.windows.net`, SharedKey auth); any "Table/database access" capability in the Source Capabilities Summary; any external service with type "managed table store" or "database".

**Cargo.toml**: no extra dependencies (types come from `omnia-sdk` re-exports). ORM requires `omnia-orm`.

## Broadcast

Send events to WebSocket clients.

|                 |                             |
| --------------- | --------------------------- |
| **Crate**       | `omnia_sdk`                 |
| **WASI module** | `omnia_wasi_websocket`      |
| **Import**      | `use omnia_sdk::Broadcast;` |

```rust
pub trait Broadcast: Send + Sync {
    fn send(
        &self, name: &str, data: &[u8], sockets: Option<Vec<String>>,
    ) -> impl Future<Output = Result<()>> + Send {
        async move {
            let client = omnia_wasi_websocket::types::Client::connect(name.to_string())
                .await
                .map_err(|e| anyhow!("connecting to websocket: {e}"))?;
            let event = omnia_wasi_websocket::types::Event::new(data);
            omnia_wasi_websocket::client::send(&client, event, sockets)
                .await
                .map_err(|e| anyhow!("sending websocket event: {e}"))
        }
    }
}
```

- `name` — WebSocket channel/connection name (passed to `Client::connect`)
- `data` — raw event payload bytes
- `sockets` — `Some(vec![socket_id, ...])` to target specific clients; `None` to broadcast to all connected clients

**Usage**:

```rust
let payload = serde_json::to_vec(&response)?;
Broadcast::send(provider, "default", &payload, None).await?;

// Target specific sockets:
Broadcast::send(provider, "channel", &data, Some(vec![socket_id])).await?;
```

**Include when**: handler sends replies over WebSocket connections. This is the Omnia equivalent of `ws.send()`. When migrating a WebSocket client that both receives and sends messages, use a WebSocket handler (for receiving) combined with `Broadcast` (for sending replies). Do NOT mark WebSocket send/reply as a missing capability.

**Specify triggers**:

- Real-time push notifications, live updates to connected clients
- WebSocket event publishing
- **WebSocket reply/response patterns** — handler receives a WebSocket event and sends data back (e.g., auth handshake responses, protocol acknowledgements, command replies)
- **Bidirectional WebSocket communication** — inbound events trigger outbound messages on the same channel
- **Protocol handshake sequences** — auth request/response, command/ack patterns over WebSocket
- Any `ws.send()`, `socket.send()`, `socket.write()`, `connection.send()`, `connection.write()`, `websocket.send`, `io.emit`, or broadcast patterns in code-analysis artifacts
- Any real-time or push delivery requirements in requirements artifacts

**Cargo.toml**: no extra dependencies.

## IntoBody

Custom response body serialization for HTTP handler output types.

|            |                            |
| ---------- | -------------------------- |
| **Crate**  | `omnia_sdk`                |
| **Import** | `use omnia_sdk::IntoBody;` |

```rust
pub trait IntoBody: Body {
    fn into_body(self) -> anyhow::Result<Vec<u8>>;
}
```

**Usage**:

```rust
use anyhow::Context as _;
use omnia_sdk::IntoBody;

impl IntoBody for DetectionReply {
    fn into_body(self) -> anyhow::Result<Vec<u8>> {
        serde_json::to_vec(&self).context("serializing DetectionReply")
    }
}
```

**Include when**: non-JSON response formats (XML, binary, plain text), custom serialization logic, or types that need explicit control over their wire representation. For standard JSON responses, `IntoBody` is not needed -- the default `Reply` serialization handles it. Messaging handlers use `type Output = ()` and do not need `IntoBody`.

**Cargo.toml**: no extra dependencies.

## Capability Selection Summary

| Specify Pattern                      | Trait         | Handler Bound                     |
| ------------------------------- | ------------- | --------------------------------- |
| Environment variables, URLs     | `Config`      | `P: Config`                       |
| Outbound HTTP calls             | `HttpRequest` | `P: HttpRequest`                  |
| Message publishing              | `Publish`     | `P: Publish`                      |
| Caching, state persistence      | `StateStore`  | `P: StateStore`                   |
| Auth tokens (Azure AD, etc.)    | `Identity`    | `P: Identity`                     |
| SQL database queries            | `TableStore`  | `P: TableStore`                   |
| Azure Table Store               | `TableStore`  | `P: TableStore`                   |
| Object-relational mapping (SQL) | `TableStore`  | `P: TableStore` (use `omnia_orm`) |
| WebSocket send/reply            | `Broadcast`   | `P: Broadcast`                    |
| HTTP response serialization     | `IntoBody`    | impl on Output type               |

**Managed data store override**: When the artifacts or source code describe direct HTTP/REST API access to a managed data store (Azure Table Storage, Azure Cosmos DB, Redis, etc.), do NOT use `HttpRequest`. Use the appropriate storage trait (`TableStore` for table/database stores, `StateStore` for key-value caches). The Omnia runtime provides native adapters for these services. Constructing raw HTTP requests with storage-specific authentication (SharedKey, HMAC-SHA256, SAS tokens) to storage service REST APIs is always wrong — the runtime handles authentication internally.

**Cache-aside / on-demand loading (TableStore + StateStore):** When the artifacts list both a database/table store (e.g. Azure Table Storage) as the source of truth and a cache for the same data, or when the legacy loads data from a data store on startup into an in-memory cache, include **both** `TableStore` (or `HttpRequest` for external APIs) and `StateStore` and implement cache-aside:
1. Read from `StateStore` (cache).
2. On miss, query the data source (`TableStore` for databases/table stores, `HttpRequest` for APIs).
3. Write the result to `StateStore` with a TTL (replacing legacy periodic refresh with TTL-based expiry).
4. Return the data.

Do **not** assume a separate cron/ETL component pre-populates the cache. The handler is self-sufficient and fetches data on demand. The legacy "load on startup" pattern becomes "load on first request" in the WASM guest.

When an HTTP call requires authentication, include **both** `HttpRequest` and `Identity`.

Handlers declare **only** the traits they actually use:

```rust
// Only needs config and HTTP
async fn handle<P>(_owner: &str, req: MyRequest, provider: &P) -> Result<Reply<MyResponse>>
where
    P: Config + HttpRequest,

// Needs config, HTTP, auth, and publishing
async fn handle<P>(_owner: &str, req: R9kMessage, provider: &P) -> Result<Reply<()>>
where
    P: Config + HttpRequest + Identity + Publish,
```

### Statelessness

WASM components must be fully stateless. All state flows through function parameters or provider trait calls.

Common methods using global singletons for managing state in Rust must be avoided (see [guardrails.md](guardrails.md)). Statefulness requirements must be met by using `StateStore` to store and retrieve information that can be cached between invocations.

## See Also

For provider composition rules, guest export patterns, and runtime setup, see the parent SKILL.md's required references list.

- [providers/README.md](providers/README.md) -- Provider bound composition rules and configuration
- [guest-patterns.md](guest-patterns.md) -- Guest export patterns (HTTP, Messaging, WebSocket)
- [runtime.md](runtime.md) -- Local development runtime setup
