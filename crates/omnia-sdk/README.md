# Omnia SDK

Shared traits, error types, and abstractions for building WASI guest components. This crate provides the glue between your business logic and the Omnia runtime capabilities.

## Quick Start

Use the `guest!` macro to define your component's API surface. This wires up the necessary WASI exports and routing logic.

```rust,ignore
use omnia_sdk::{guest, Handler, Json};
use serde::{Deserialize, Serialize};

// Define your data models
#[derive(Deserialize)]
struct CreateItem {
    name: String,
}

#[derive(Serialize)]
struct ItemResponse {
    id: String,
    name: String,
}

// Define the provider (capabilities your app needs)
struct MyProvider;

// Wire up the application
guest!({
    owner: "my-org",
    provider: MyProvider,
    http: [
        "/items": post(CreateItem with_body, ItemResponse),
    ],
});

// Implement the handler for the request
impl Handler<MyProvider> for CreateItem {
    type Response = ItemResponse;

    async fn handle(self, _provider: &MyProvider) -> Result<Self::Response, omnia_sdk::Error> {
        Ok(ItemResponse {
            id: "123".to_string(),
            name: self.name,
        })
    }
}
```

## Capabilities

The SDK exposes trait-based abstractions for host capabilities. When compiled to `wasm32`, these delegate to WASI host calls.

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
use omnia_sdk::{StateStore, Publish, Message};

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
use omnia_sdk::{bad_request, server_error, not_found};

fn validate(name: &str) -> Result<(), omnia_sdk::Error> {
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
