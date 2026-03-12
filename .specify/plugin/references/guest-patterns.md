# Guest Patterns

Canonical reference for HTTP, Messaging, and WebSocket guest export structs, trait implementations, and handler invocation patterns in WASM guests.

---

## HTTP Handler Setup

Axum routing and HTTP endpoint wiring for WASM guests.

### HTTP Guest Export

The HTTP struct must implement `wasip3::exports::http::handler::Guest`:

```rust
#![cfg(target_arch = "wasm32")]

use axum::Router;
use axum::extract::{Path, RawQuery};
use axum::routing::{get, post};
use bytes::Bytes;
use domain_crate_a::{EndpointRequest, EndpointResponse};
use domain_crate_b::{SingleParamRequest, SingleParamResponse};
use domain_crate_c::{MultipleParamsRequest, MultipleParamsResponse};
use domain_crate_d::{QueryRequest, QueryResponse};
use domain_crate_e::{PostRequest, PostResponse};
use domain_crate_f::{UpdateItemRequest, UpdateItemResponse};
use omnia_sdk::{Handler, HttpResult, Reply};
use tracing::Level;
use wasip3::exports::http::handler::Guest;
use wasip3::http::types::{ErrorCode, Request, Response};

struct Http;
wasip3::http::proxy::export!(Http);

impl Guest for Http {
    #[omnia_wasi_otel::instrument(name = "http_guest_handle")]
    async fn handle(request: Request) -> HttpResult<Response, ErrorCode> {
        let router = Router::new()
            .route("/endpoint", get(endpoint))
            .route("/params/{id}", get(single_param))
            .route("/params/{id1}/{id2}", get(multiple_params))
            .route("/query", get(query))
            .route("/post", post(post_handler))
            .route("/items/{id}", post(update_item));
        omnia_wasi_http::serve(router, request).await
    }
}
```

### Handler Patterns

| Pattern           | Signature                                                                        | Use Case                     |
| ----------------- | -------------------------------------------------------------------------------- | ---------------------------- |
| No body           | `async fn handler() -> &'static str`                                             | Health checks                |
| No arguments      | `async fn handler() -> HttpResult<Reply<T>>`                                     | GET/POST with no input       |
| JSON body         | `async fn handler(body: Bytes) -> HttpResult<Reply<T>>`                          | POST/PUT with body           |
| Path param        | `async fn handler(Path(id): Path<String>) -> HttpResult<Reply<T>>`               | GET by ID                    |
| Tuple path params | `async fn handler(Path((a, b)): Path<(String, String)>) -> HttpResult<Reply<T>>` | Multiple path segments       |
| Raw query         | `async fn handler(RawQuery(q): RawQuery) -> HttpResult<Reply<T>>`                | Raw query string passthrough |
| Query param       | `async fn handler(Query(params): Query<T>) -> HttpResult<Reply<T>>`              | Typed query extraction       |
| Body + path param | `async fn handler(Path(id): Path<String>, body: Bytes) -> HttpResult<Reply<T>>`  | PUT/POST by ID with body     |

#### Endpoint only (no path params, query, or body)

Route: `.route("/endpoint", get(endpoint))`

```rust
#[omnia_wasi_otel::instrument]
async fn endpoint() -> HttpResult<Reply<EndpointResponse>> {
    EndpointRequest::handler(())?
        .provider(&Provider::new())
        .owner("owner")
        .await
        .map_err(Into::into)
}
```

#### GET with single path parameter

Route: `.route("/params/{id}", get(single_param))`

```rust
#[omnia_wasi_otel::instrument]
async fn single_param(Path(id): Path<String>) -> HttpResult<Reply<SingleParamResponse>> {
    SingleParamRequest::handler(id)?
        .provider(&Provider::new())
        .owner("owner")
        .await
        .map_err(Into::into)
}
```

#### GET with multiple path parameters

Route: `.route("/params/{id1}/{id2}", get(multiple_params))`

```rust
#[omnia_wasi_otel::instrument]
async fn multiple_params(Path((id1, id2)): Path<(String, String)>) -> HttpResult<Reply<MultipleParamsResponse>> {
    MultipleParamsRequest::handler((id1, id2))?
        .provider(&Provider::new())
        .owner("owner")
        .await
        .map_err(Into::into)
}
```

#### GET with query parameters

Route: `.route("/query", get(query))`

```rust
#[omnia_wasi_otel::instrument]
async fn query(RawQuery(query): RawQuery) -> HttpResult<Reply<QueryResponse>> {
    QueryRequest::handler(query)?
        .provider(&Provider::new())
        .owner("owner")
        .await
        .map_err(Into::into)
}
```

#### POST with body

Route: `.route("/post", post(post_handler))`

```rust
#[omnia_wasi_otel::instrument]
async fn post_handler(body: Bytes) -> HttpResult<Reply<PostResponse>> {
    PostRequest::handler(body.to_vec())?
        .provider(&Provider::new())
        .owner("owner")
        .await
        .map_err(Into::into)
}
```

#### POST/PUT with body and path parameter

Route: `.route("/items/{id}", post(update_item))`

```rust
#[omnia_wasi_otel::instrument]
async fn update_item(Path(id): Path<String>, body: Bytes) -> HttpResult<Reply<UpdateItemResponse>> {
    UpdateItemRequest::handler((id, body.to_vec()))?
        .provider(&Provider::new())
        .owner("owner")
        .await
        .map_err(Into::into)
}
```

### Handler Invocation Pattern

All domain handlers follow the builder API:

```rust
RequestType::handler(input)?      // Parse input, returns Result
    .provider(&Provider::new())   // Attach provider
    .owner("owner")               // Set owner (required)
    .await                        // Execute handler
    .map_err(Into::into)          // Convert error to HttpError
```

- **`owner`** -- hardcoded string identifying the Omnia component owner. See [providers/README.md](providers/README.md#owner).
- **Return type** -- `HttpResult<Reply<T>>` where `T` is the response type. The `Reply` wrapper is required.

### Handler Instrumentation

Annotate individual handler functions with `#[omnia_wasi_otel::instrument]` for per-handler tracing spans:

```rust
#[omnia_wasi_otel::instrument]
async fn endpoint() -> HttpResult<Reply<EndpointResponse>> {
    // ...
}
```

This creates a tracing span named after the function. The `Guest for Http` impl uses a named span (`name = "http_guest_handle"`) for the top-level dispatch; individual handlers use the default (function name).

### Route Parameters

Use `{param}` brace syntax (Axum 0.8), **not** `:param` colon syntax:

```rust
// Correct (Axum 0.8)
.route("/api/users/{user_id}", get(get_user))

// Wrong (Axum 0.7 -- do not use)
// .route("/api/users/:user_id", get(get_user))
```

### Route Methods

| Method     | Usage             |
| ---------- | ----------------- |
| `get()`    | Read operations   |
| `post()`   | Create operations |
| `put()`    | Update operations |
| `delete()` | Delete operations |

### HTTP Error Handling

Use `HttpResult` with error macros:

```rust
use omnia_sdk::{HttpResult, Reply, bad_request};

async fn handler() -> HttpResult<Reply<Response>> {
    if invalid {
        return Err(bad_request!("validation failed: {}", reason));
    }
    // ...
}
```

---

## Message Handler Setup

Message subscriptions and publishing in WASM guests.

### Messaging Guest Export

The Messaging struct must implement `omnia_wasi_messaging::incoming_handler::Guest`:

```rust
#![cfg(target_arch = "wasm32")]

use omnia_wasi_messaging::types::{Error, Message};
use domain_crate_a::{Topic1Request, Topic1Response};
use domain_crate_b::{Topic2Request, Topic2Response};
use tracing::Level;
use omnia_wasi_messaging::incoming_handler::Guest;

pub struct Messaging;
omnia_wasi_messaging::export!(Messaging with_types_in omnia_wasi_messaging);

impl Guest for Messaging {
    #[omnia_wasi_otel::instrument(name = "messaging_guest_handle")]
    async fn handle(message: Message) -> Result<(), Error> {
        if let Err(e) = match &message.topic().unwrap_or_default() {
            t if t.contains("topic-pattern-1") => topic1(message.data()).await,
            t if t.contains("topic-pattern-2") => topic2(message.data()).await,
            _ => {
                return Err(Error::Other("Unhandled topic".to_string()));
            }
        } {
            return Err(Error::Other(e.to_string()));
        }
        Ok(())
    }
}
```

### Message Handler Pattern

Message handlers use the same builder API as HTTP handlers:

```rust
#[omnia_wasi_otel::instrument]
async fn topic1(payload: Vec<u8>) -> anyhow::Result<()> {
    Topic1Request::handler(payload)?
        .provider(&Provider::new())
        .owner("owner")
        .await
        .map(|_| ())
        .map_err(Into::into)
}
```

**Note:** `.map(|_| ())` discards the reply since messaging handlers return `Result<()>`.

### Unhandled Topics

Unhandled topics **must return an error**, not silently succeed:

```rust
_ => {
    return Err(Error::Other("Unhandled topic".to_string()));
}
```

This ensures the messaging infrastructure knows the message was not processed and can route it to a dead-letter queue.

### Publishing Messages

#### Via Provider Trait

```rust
async fn publish_notification<P: Publish>(provider: &P, notification: &NotificationEvent) -> anyhow::Result<()> {
    let payload = serde_json::to_vec(notification)?;
    let message = Message::new(&payload);
    Publish::send(provider, "notifications.v1", &message).await
}
```

### Topic Naming Convention

```
<domain>-<entity>-<event-type>.v<version>

Examples: user-events.v1, order-created.v1, payment-processed.v2
```

### Topic Routing

| Pattern        | Example                             | When to use                 |
| -------------- | ----------------------------------- | --------------------------- |
| Exact match    | `"user-events.v1" =>`               | Known exact topic names     |
| Contains match | `t if t.contains("user-events") =>` | Environment-prefixed topics |

### Messaging Error Handling

Always convert errors to `Error::Other`:

```rust
if let Err(e) = process_message(&message).await {
    tracing::error!(error = %e, "message processing failed");
    return Err(Error::Other(e.to_string()));
}
```

---

## WebSocket Handler Setup

WebSocket event handling for WASM guests. WebSocket handlers receive inbound events from connected clients and optionally send events back.

### WebSocket Guest Export

The WebSocket struct must implement `omnia_wasi_websocket::incoming_handler::Guest`:

```rust
pub struct WebSocket;
omnia_wasi_websocket::export!(WebSocket);

impl omnia_wasi_websocket::incoming_handler::Guest for WebSocket {
    #[omnia_wasi_otel::instrument(name = "websocket_guest_handle")]
    async fn handle(event: Event) -> Result<(), WsError> {
        handle_ws_event(event.data()).await
            .map_err(|e| WsError::Other(e.to_string()))
    }
}
```

### WebSocket Handler Pattern

WebSocket handlers use the same builder API as HTTP and messaging handlers:

```rust
#[omnia_wasi_otel::instrument]
async fn handle_ws_event(payload: Vec<u8>) -> anyhow::Result<()> {
    WebSocketEventRequest::handler(payload)?
        .provider(&Provider::new())
        .owner("owner")
        .await
        .map(|_| ())
        .map_err(Into::into)
}
```

**Note:** `.map(|_| ())` discards the reply since WebSocket handlers return `Result<()>`, same as messaging handlers.

### Sending Events to WebSocket Clients

Use `Client::connect` and `client::send` to push events back to connected clients:

```rust
use omnia_wasi_websocket::client;
use omnia_wasi_websocket::types::{Client, Event};

let client = Client::connect("default".to_string()).await
    .map_err(|e| anyhow!("connecting: {e}"))?;
let event = Event::new(&payload);
client::send(&client, event, None).await
    .map_err(|e| anyhow!("sending event: {e}"))?;
```

- First argument to `Client::connect` is the channel name (e.g., `"default"`)
- `client::send` third argument: `None` broadcasts to all clients; `Some(vec![...])` targets specific socket IDs

For domain crate code that needs to send to WebSocket clients, use the `Broadcast` capability trait instead of calling `omnia_wasi_websocket` directly. See [capabilities.md](capabilities.md#broadcast).

### WebSocket Error Handling

Convert errors to `omnia_wasi_websocket::types::Error::Other`:

```rust
if let Err(e) = process_event(&event).await {
    tracing::error!(error = %e, "websocket event processing failed");
    return Err(WsError::Other(e.to_string()));
}
```

When both messaging and WebSocket error types are in scope, alias one to avoid collision:

```rust
use omnia_wasi_messaging::types::{Error, Message};
use omnia_wasi_websocket::types::{Error as WsError, Event};
```

---

## Key Points

1. **wasm32 guard** -- `#![cfg(target_arch = "wasm32")]` at top of file
2. **HTTP export** -- `wasip3::http::proxy::export!(Http);`
3. **Messaging export** -- `omnia_wasi_messaging::export!(Messaging with_types_in omnia_wasi_messaging);`
4. **WebSocket export** -- `omnia_wasi_websocket::export!(WebSocket);`
5. **Handler builder API** -- `Type::handler(input)?.provider(&provider).owner("owner").await`
6. **Owner** -- hardcoded string identifying the Omnia component owner
7. **Reply wrapper** -- HTTP handlers return `HttpResult<Reply<T>>`, not `HttpResult<T>`
8. **Instrumentation** -- `#[omnia_wasi_otel::instrument]` for tracing
9. **Unhandled topics** -- return `Err(Error::Other(...))`, not `Ok(())`
10. **Route params** -- use `{param}` syntax (Axum 0.8), not `:param`
11. **WebSocket error alias** -- when both messaging and WebSocket errors are in scope, alias WebSocket's as `WsError`
12. **Capability Provider** -- see [guest-wiring.md](guest-wiring.md#capability-provider) for the `Provider` struct and trait implementations

## References

- [guest-wiring.md](guest-wiring.md) -- Crate injection procedure and Provider definition
- [capabilities.md](capabilities.md) -- WASI capability trait definitions
- [providers/README.md](providers/README.md) -- Provider configuration and composition
- [runtime.md](runtime.md) -- Local development runtime setup
