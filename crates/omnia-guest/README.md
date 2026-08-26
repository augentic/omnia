# Omnia Guest

Shared traits, error types, and abstractions for building WASI guest components. This crate provides the glue between your business logic and the Omnia runtime capabilities.

## Quick Start

Implement `Handler` on an input type, then register it with an explicit transport router.

```rust,ignore
use omnia_guest::api::http::{Router, post};
use omnia_guest::api::{Client, Context, Handler};
use omnia_guest::Error;
use serde::{Deserialize, Serialize};

#[derive(Deserialize)]
struct CreateItem {
    name: String,
}

#[derive(Debug, Serialize)]
struct ItemResponse {
    id: String,
    name: String,
}

struct MyProvider;

impl Handler<MyProvider> for CreateItem {
    type Output = ItemResponse;
    type Error = Error;

    async fn handle(
        self,
        _context: Context<'_, MyProvider>,
    ) -> Result<Self::Output, Self::Error> {
        Ok(ItemResponse {
            id: "123".to_string(),
            name: self.name,
        })
    }
}

fn router() -> Router<MyProvider> {
    Router::new(Client::new("my-org", MyProvider))
        .route("/items", post::<CreateItem, MyProvider>())
}
```

`Client` owns the provider and supplies `Context` (owner, provider, metadata) when it calls the handler. The application owns its WASI export explicitly:

```rust,ignore
struct Http;
wasip3::http::service::export!(Http);

impl wasip3::exports::http::handler::Guest for Http {
    async fn handle(request: Request) -> Result<Response, ErrorCode> {
        omnia_guest::api::http::serve(router(), request).await
    }
}
```

Omnia creates one WASI component instance per HTTP request. Construct one `Router` with one provider-owning `Client` inside each `handle` call; Axum's route-state clones share that client's `Arc<P>` only for that request. Durable application state belongs in host-side capabilities, not guest statics.

Messaging routes use the same handlers with exact topic registration:

```rust,ignore
use omnia_guest::api::messaging::{Router, consume};

let router = Router::new(Client::new("my-org", MyProvider))
    .route("items.created", consume::<CreateItem>());
```

`consume` decodes JSON and acknowledges successful output.

Command-mode guests parse argv with clap and call `Client::call` on the same handlers. Wrap the clap dispatch in `command::execute_wasi` so guest telemetry is initialized and flushed. Omnia creates a fresh component instance for each command invocation.

## Capabilities

The guest crate exposes trait-based abstractions for host capabilities. When compiled to `wasm32`, these delegate to WASI host calls.

| Trait | Purpose |
| ----- | ------- |
| `Config` | Read configuration values from the host. |
| `HttpRequest` | Make outbound HTTP requests. |
| `Publish` | Publish messages to a topic. |
| `StateStore` | Get/set/delete key-value state with optional TTL, plus one-shot `cas` (conflict is a typed `CasError::Conflict`) and atomic `increment`. |
| `Identity` | Obtain access tokens from an identity provider. |
| `TableStore` | Execute SQL queries and statements via the ORM layer. |
| `Broadcast` | Send events over WebSocket channels. |

### Example: Using Capabilities

```rust,ignore
use omnia_guest::{StateStore, Publish, Message};

async fn process(provider: &impl StateStore + Publish) -> anyhow::Result<()> {
    // Store some state
    provider.set("last_run", b"now", None).await?;

    // Publish a message
    let msg = Message::new(b"job_completed");
    provider.send("jobs.events", &msg).await?;

    Ok(())
}
```

## Error Handling

The crate provides an `Error` enum with HTTP-aware variants (`BadRequest`, `NotFound`, `ServerError`, `BadGateway`) and helper macros for ergonomic error creation.

```rust,ignore
use omnia_guest::{bad_request, server_error, not_found};

fn validate(name: &str) -> Result<(), omnia_guest::Error> {
    if name.is_empty() {
        return Err(bad_request!("name cannot be empty"));
    }
    Ok(())
}
```

## Architecture

See the [workspace documentation](https://github.com/augentic/omnia) for the full architecture guide.

## Cargo features

- `orm` *(default)*: the SQL ORM, table/document capabilities, and document-store re-exports.

Guests that do not use SQL or documents can disable defaults to shrink wasm build time and size.

## License

MIT OR Apache-2.0
