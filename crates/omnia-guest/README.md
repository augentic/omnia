# Omnia Guest

Shared traits, error types, and abstractions for building WASI guest components. This crate provides the glue between your business logic and the Omnia runtime capabilities.

## Quick Start

Define transport-neutral operations, then register them with an explicit transport router.

```rust,ignore
use omnia_guest::api::http::{Router, post};
use omnia_guest::api::{CallContext, Invoker, Operation};
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
struct Create;

impl Operation<MyProvider> for Create {
    type Input = CreateItem;
    type Output = ItemResponse;
    type Error = Error;

    async fn call(
        input: Self::Input,
        _context: CallContext<'_, MyProvider>,
    ) -> Result<Self::Output, Self::Error> {
        Ok(ItemResponse {
            id: "123".to_string(),
            name: input.name,
        })
    }
}

fn router() -> Router<MyProvider> {
    Router::new(Invoker::new("my-org", MyProvider))
        .route("/items", post::<Create, MyProvider>())
}
```

`Invocation<Input>` carries typed input plus transport-neutral metadata. The router creates it, and `Invoker` owns the provider and supplies `CallContext` when it calls the operation. The application owns its WASI export explicitly:

```rust,ignore
struct Http;
wasip3::http::service::export!(Http);

impl wasip3::exports::http::handler::Guest for Http {
    async fn handle(request: Request) -> Result<Response, ErrorCode> {
        omnia_guest::api::http::serve(router(), request).await
    }
}
```

Omnia creates one WASI component instance per HTTP request. Construct one `Router` with one provider-owning `Invoker` inside each `handle` call; Axum's route-state clones share that invoker's `Arc<P>` only for that request. Durable application state belongs in host-side capabilities, not guest statics.

Messaging routes use the same operations with exact topic registration:

```rust,ignore
use omnia_guest::api::messaging::{Router, consume};

let router = Router::new(Invoker::new("my-org", MyProvider))
    .route("items.created", consume::<Create>());
```

`consume` decodes JSON and acknowledges successful output by default. `decode_with` and `project_with` make payload and delivery policy explicit when those defaults do not fit.

Command routes use the same operations with Clap-derived arguments through `omnia_guest::api::command`. Build a `Router` explicitly inside the component's `wasi:cli/run` implementation, then call `command::execute_wasi`; `command::run::<Args, Operation>()` remains the distinct typed route builder. Omnia creates a fresh component instance for each command invocation, so no static router is needed.

## Capabilities

The guest crate exposes trait-based abstractions for host capabilities. When compiled to `wasm32`, these delegate to WASI host calls.

| Trait | Purpose |
| ----- | ------- |
| `Config` | Read configuration values from the host. |
| `HttpRequest` | Make outbound HTTP requests. |
| `Publish` | Publish messages to a topic. |
| `StateStore` | Get/set/delete key-value state with optional TTL. |
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

## License

MIT OR Apache-2.0
