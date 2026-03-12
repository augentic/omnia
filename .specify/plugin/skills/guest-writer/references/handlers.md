# Handlers

HTTP routing, message subscriptions, WebSocket events, lib.rs wiring, and the `guest!` macro for WASM guests.

For the canonical export patterns (struct definitions, trait implementations, handler invocation patterns, and error handling), see [guest-patterns.md](omnia/guest-patterns.md).

---

## Complete lib.rs Example

Full working example of a WASM guest module.

### Manual Implementation

```rust
#![cfg(target_arch = "wasm32")]
//! WASM Guest for User Service
//!
//! Wraps the user-domain crate and exposes:
//! - HTTP endpoints for user CRUD operations
//! - Message topic subscriptions for user events
//! - WebSocket event handling for real-time updates

use anyhow::Result;
use axum::Router;
use axum::extract::Path;
use axum::routing::{get, post, put};
use bytes::Bytes;
use omnia_sdk::{Config, Handler, HttpRequest, HttpResult, Identity, Publish, Reply, StateStore, ensure_env};
use omnia_wasi_messaging::types::{Error, Message};
use omnia_wasi_websocket::types::{Error as WsError, Event};
use tracing::Level;
use wasip3::exports::http::handler::Guest;
use wasip3::http::types as p3;

// Import domain crate handlers and types
use user_domain::{
    CreateUserRequest, CreateUserResponse,
    GetUserRequest, GetUserResponse,
    UpdateUserRequest, UpdateUserResponse,
    UserCreatedEvent, UserUpdatedEvent,
    UserNotification,
};

// ============================================================================
// HTTP Handler
// ============================================================================

pub struct Http;
wasip3::http::proxy::export!(Http);

impl Guest for Http {
    #[omnia_wasi_otel::instrument(name = "http_guest_handle", level = Level::INFO)]
    async fn handle(request: p3::Request) -> Result<p3::Response, p3::ErrorCode> {
        let router = Router::new()
            .route("/api/users", post(create_user))
            .route("/api/users/{user_id}", get(get_user))
            .route("/api/users/{user_id}", put(update_user));

        omnia_wasi_http::serve(router, request).await
    }
}

async fn create_user(body: Bytes) -> HttpResult<Reply<CreateUserResponse>> {
    CreateUserRequest::handler(body.to_vec())?
        .provider(&Provider::new())
        .owner("at")
        .await
        .map_err(Into::into)
}

async fn get_user(Path(user_id): Path<String>) -> HttpResult<Reply<GetUserResponse>> {
    GetUserRequest::handler(user_id)?
        .provider(&Provider::new())
        .owner("at")
        .await
        .map_err(Into::into)
}

async fn update_user(
    Path(user_id): Path<String>,
    body: Bytes,
) -> HttpResult<Reply<UpdateUserResponse>> {
    UpdateUserRequest::handler((user_id, body.to_vec()))?
        .provider(&Provider::new())
        .owner("at")
        .await
        .map_err(Into::into)
}

// ============================================================================
// Messaging Handler
// ============================================================================

pub struct Messaging;
omnia_wasi_messaging::export!(Messaging with_types_in omnia_wasi_messaging);

impl omnia_wasi_messaging::incoming_handler::Guest for Messaging {
    #[omnia_wasi_otel::instrument(name = "messaging_guest_handle")]
    async fn handle(message: Message) -> Result<(), Error> {
        if let Err(e) = match &message.topic().unwrap_or_default() {
            t if t.contains("user-created.v1") => {
                handle_user_created(message.data()).await
            }
            t if t.contains("user-updated.v1") => {
                handle_user_updated(message.data()).await
            }
            _ => {
                return Err(Error::Other("Unhandled topic".to_string()));
            }
        } {
            return Err(Error::Other(e.to_string()));
        }
        Ok(())
    }
}

#[omnia_wasi_otel::instrument]
async fn handle_user_created(payload: Vec<u8>) -> Result<()> {
    UserCreatedEvent::handler(payload)?
        .provider(&Provider::new())
        .owner("at")
        .await
        .map(|_| ())
        .map_err(Into::into)
}

#[omnia_wasi_otel::instrument]
async fn handle_user_updated(payload: Vec<u8>) -> Result<()> {
    UserUpdatedEvent::handler(payload)?
        .provider(&Provider::new())
        .owner("at")
        .await
        .map(|_| ())
        .map_err(Into::into)
}

// ============================================================================
// WebSocket Handler
// ============================================================================

struct WebSocketGuest;
omnia_wasi_websocket::export!(WebSocketGuest);

impl omnia_wasi_websocket::incoming_handler::Guest for WebSocketGuest {
    #[omnia_wasi_otel::instrument(name = "websocket_guest_handle")]
    async fn handle(event: Event) -> Result<(), WsError> {
        handle_user_notification(event.data()).await
            .map_err(|e| WsError::Other(e.to_string()))
    }
}

#[omnia_wasi_otel::instrument]
async fn handle_user_notification(payload: Vec<u8>) -> Result<()> {
    UserNotification::handler(payload)?
        .provider(&Provider::new())
        .owner("at")
        .await
        .map(|_| ())
        .map_err(Into::into)
}

// ============================================================================
// Provider Configuration
// ============================================================================

#[derive(Clone, Default)]
pub struct Provider;

impl Provider {
    #[must_use]
    pub fn new() -> Self {
        ensure_env!(
            "API_URL",
            "SERVICE_NAME",
            "AZURE_IDENTITY",
        );
        Self
    }
}

impl Config for Provider {}
impl HttpRequest for Provider {}
impl Identity for Provider {}
impl Publish for Provider {}
impl StateStore for Provider {}
```

### Macro Implementation

The same guest can be generated using the `omnia_sdk::guest!` macro, which replaces the manual HTTP, Messaging, and WebSocket wiring with a declarative DSL.

```rust
#![cfg(target_arch = "wasm32")]

use user_domain::{
    CreateUserRequest, CreateUserResponse,
    GetUserRequest, GetUserResponse,
    UpdateUserRequest, UpdateUserResponse,
    UserCreatedEvent, UserUpdatedEvent,
    UserNotification,
};
use omnia_sdk::{Config, HttpRequest, Identity, Publish, StateStore, ensure_env};

omnia_sdk::guest!({
    owner: "at",
    provider: Provider,
    http: [
        "/api/users": post(CreateUserRequest with_body, CreateUserResponse),
        "/api/users/{user_id}": get(GetUserRequest, GetUserResponse),
        "/api/users/{user_id}": put(UpdateUserRequest with_body, UpdateUserResponse),
    ],
    messaging: [
        "user-created.v1": UserCreatedEvent,
        "user-updated.v1": UserUpdatedEvent,
    ],
    websocket: [
        "default": UserNotification,
    ]
});

#[derive(Clone, Default)]
pub struct Provider;

impl Provider {
    #[must_use]
    pub fn new() -> Self {
        ensure_env!("API_URL", "SERVICE_NAME", "AZURE_IDENTITY");
        Self
    }
}

impl Config for Provider {}
impl HttpRequest for Provider {}
impl Identity for Provider {}
impl Publish for Provider {}
impl StateStore for Provider {}
```

### Key Points

1. **wasm32 guard** -- `#![cfg(target_arch = "wasm32")]` at top of file
2. **HTTP export** -- `wasip3::http::proxy::export!(Http);`
3. **Messaging export** -- `omnia_wasi_messaging::export!(Messaging with_types_in omnia_wasi_messaging);`
4. **WebSocket export** -- `omnia_wasi_websocket::export!(WebSocketGuest);`
5. **Handler builder API** -- `Type::handler(input)?.provider(&provider).owner("owner").await`
6. **Owner** -- hardcoded string identifying the Omnia component owner (e.g. `"at"`)
7. **Reply wrapper** -- HTTP handlers return `HttpResult<Reply<T>>`, not `HttpResult<T>`
8. **Provider validation** -- `ensure_env!` validates required config at startup (optional)
9. **Instrumentation** -- `#[omnia_wasi_otel::instrument]` for tracing
10. **Unhandled topics** -- return `Err(Error::Other(...))`, not `Ok(())`
11. **Route params** -- use `{param}` syntax (Axum 0.8), not `:param`
12. **WebSocket error alias** -- when both messaging and WebSocket errors are in scope, alias WebSocket's as `WsError`

For detailed handler patterns, route methods, error handling, and individual handler signatures, see [guest-patterns.md](omnia/guest-patterns.md).

---

## `guest!` Macro

Declarative macro that replaces manual HTTP, Messaging, and WebSocket guest wiring with a concise DSL. Generates the `Http` struct, `Messaging` struct, `WebSocketGuest` struct, export macros, Axum router, topic dispatcher, WebSocket event handler, and all handler functions.

### When to Use

- **Use the macro** for standard guests where handlers follow the builder API pattern
- **Use manual wiring** when you need custom middleware, pre/post-processing, or non-standard handler signatures

### Syntax

```rust
omnia_sdk::guest!({
    owner: "<owner-string>",
    provider: <ProviderType>,
    http: [
        "<path>": <method>(<RequestType> [with_body|with_query], <ResponseType>),
        // ... more routes
    ],
    messaging: [
        "<topic-pattern>": <MessageType>,
        // ... more topics
    ],
    websocket: [
        "<channel-name>": <EventType>,
        // ... more channels
    ]
});
```

### Fields

#### `owner` (required)

Hardcoded string identifying the Omnia component owner. Passed to every handler invocation via `.owner("...")`.

```rust
owner: "at",
```

#### `provider` (required)

The Provider struct type. Must implement the required Omnia SDK traits (`Config`, `HttpRequest`, etc.) and have a `::new()` constructor.

```rust
provider: Provider,
```

#### `http` (optional)

Array of HTTP route definitions. Each entry maps a path and method to a request/response type pair.

```rust
http: [
    "/api/apc": post(DilaxRequest with_body, DilaxReply),
    "/jobs/detector": get(DetectionRequest, DetectionReply),
    "/info/{vehicle_id}": get(VehicleInfoRequest, VehicleInfoReply),
    "/worksite": get(WorksiteRequest with_query, WorksiteResponse),
],
```

- **Path** -- Axum route path using `{param}` brace syntax for parameters
- **Method** -- `get`, `post`, `put`, `delete`
- **`with_body`** -- append after the request type for handlers that receive `Bytes` body (typically POST/PUT)
- **`with_query`** -- append after the request type for handlers that receive the raw query string via `RawQuery` (typically GET with query parameters)
- **Request type** -- domain crate type implementing `Handler`
- **Response type** -- the handler's output type

#### `messaging` (optional)

Array of topic-to-handler mappings. Each entry maps a topic pattern to a message type.

```rust
messaging: [
    "realtime-r9k.v1": R9kMessage,
    "realtime-dilax-apc.v2": DilaxMessage,
],
```

- **Topic pattern** -- matched using `contains()` to support environment-prefixed topics
- **Message type** -- domain crate type implementing `Handler`

#### `websocket` (optional)

Array of channel-to-handler mappings. Each entry maps a WebSocket channel name to an event type.

```rust
websocket: [
    "default": PositionEvent,
],
```

- **Channel name** -- passed to `Client::connect` to identify the WebSocket channel
- **Event type** -- domain crate type implementing `Handler`

### What the Macro Generates

The macro expands to approximately the following manual code:

1. **HTTP struct + export** -- `pub struct Http;` with `wasip3::http::proxy::export!(Http);`
2. **`Guest` implementation** -- Axum `Router` with all routes from the `http:` block
3. **Handler functions** -- one per route, using the builder API:
   ```rust
   RequestType::handler(input)?
       .provider(&Provider::new())
       .owner("at")
       .await
       .map_err(Into::into)
   ```
4. **Messaging struct + export** -- `pub struct Messaging;` with topic dispatcher
5. **Topic handlers** -- one per topic, using the builder API with `.map(|_| ())`
6. **WebSocket struct + export** -- `struct WebSocketGuest;` with event handler (if `websocket:` block present)
7. **WebSocket handlers** -- one per channel, using the builder API with `.map(|_| ())`
8. **Health check** -- automatic `/health` endpoint returning `"OK"`

### Choosing Between Macro and Manual

| Consideration            | Macro | Manual   |
| ------------------------ | ----- | -------- |
| Standard handler pattern | Yes   | Yes      |
| Custom middleware        | No    | Yes      |
| Pre/post-processing      | No    | Yes      |
| Non-standard signatures  | No    | Yes      |
| Lines of code            | ~30   | ~150     |
| Readable at a glance     | Yes   | Moderate |
