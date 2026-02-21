# Omnia SDK

Shared traits, error types, and abstractions consumed by WASM guests and domain crates. These definitions provide the capability contracts used for dependency injection across adapters.

## Quick Start

Use the `guest!` macro to wire up HTTP routes and messaging handlers for a WASI component guest:

```rust,ignore
omnia_sdk::guest!({
    owner: "my-org",
    provider: MyProvider,
    http: [
        "/items": get(ListItems, ItemsResponse),
        "/items": post(CreateItem with_body, ItemResponse),
    ],
    messaging: [
        "order-placed.v1": OrderPlaced,
    ]
});
```

The macro generates the WASI HTTP handler, axum router, and messaging subscriber glue so your business logic only needs to implement the `Handler` trait for each request type.

## Capabilities

The SDK exposes trait-based abstractions that are automatically backed by WASI host calls when compiled to `wasm32`:

| Trait | Purpose |
|-------|---------|
| `Config` | Read configuration values from the host |
| `HttpRequest` | Make outbound HTTP requests |
| `Publish` | Publish messages to a topic |
| `StateStore` | Get/set/delete key-value state with optional TTL |
| `Identity` | Obtain access tokens from an identity provider |
| `TableStore` | Execute SQL queries and statements via the ORM layer |
| `Broadcast` | Send events over WebSocket channels |

When targeting `wasm32`, each trait has a default implementation that delegates to the corresponding `omnia-wasi-*` bindings. Host-side test code can provide mock implementations by implementing the same traits.

## Error Handling

The crate provides an `Error` enum with HTTP-aware variants (`BadRequest`, `NotFound`, `ServerError`, `BadGateway`) plus helper macros:

```rust,ignore
use omnia_sdk::{bad_request, server_error};

let err = bad_request!("missing field: {}", "name");
```

## Architecture

See the [workspace documentation](https://github.com/augentic/omnia) for the full architecture guide.

## License

MIT OR Apache-2.0
