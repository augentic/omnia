# guest-macro

Procedural macros for generating WebAssembly guest infrastructure.

## Overview

This crate provides the `guest!` macro that generates the necessary guest infrastructure for WebAssembly components with WASI capabilities. Instead of manually implementing HTTP handlers and messaging consumers, you declaratively specify your routes and topics.

## Usage

Add `guest-macro` to your dependencies:

```toml
[dependencies]
guest-macro = { workspace = true }
```

Then use the `guest!` macro to generate your guest infrastructure:

```rust,ignore
use guest_macro::guest;

guest!({
    owner: "my-org",
    provider: MyProvider,
    http: [
        "/api/users": get(GetUsersRequest, GetUsersResponse),
        "/api/users": post(CreateUserRequest with_body, CreateUserResponse),
        "/api/search": get(SearchRequest with_query, SearchResponse),
        "/api/users/{user_id}": get(GetUserRequest, GetUserResponse),
    ],
    messaging: [
        "user-events.v1": UserEventMessage,
        "notifications.v1": NotificationMessage,
    ]
});
```

## Configuration Format

The macro accepts a struct-like syntax with the following fields:

### Required Fields

- **`owner`**: A string literal identifying the owner/organization
- **`provider`**: An identifier for the provider type that implements the necessary traits

### Optional Fields

- **`http`**: An array of HTTP route definitions
- **`messaging`**: An array of messaging topic definitions

## HTTP Routes

HTTP routes are defined with the syntax:

```rust,ignore
"/path": method(RequestType, ResponseType)
```

### Supported Methods

- `get(Request, Response)` - GET request handler
- `post(Request, Response)` - POST request handler

### Request Modifiers

- `with_query` - For GET requests, indicates the request type should be populated from query parameters
- `with_body` - For POST requests, indicates the request should include the raw body bytes

### Path Parameters

Path parameters use curly brace syntax and are automatically extracted:

```rust,ignore
"/users/{user_id}/posts/{post_id}": get(GetPostRequest, GetPostResponse)
```

## Messaging Topics

Messaging topics are defined with the syntax:

```rust,ignore
"topic-name.version": MessageType
```

The macro generates handlers that match incoming messages by topic name.

## Generated Code

The macro generates the following modules under `#[cfg(target_arch = "wasm32")]`:

### HTTP Module (`mod http`)

- Implements `wasip3::exports::http::handler::Guest` trait
- Sets up an Axum router with all defined routes
- Generates async handler functions for each route with OpenTelemetry instrumentation

### Messaging Module (`mod messaging`)

- Implements `wasi_messaging::incoming_handler::Guest` trait
- Routes incoming messages to appropriate handlers based on topic
- Generates async processor functions with OpenTelemetry instrumentation

## Example

```rust,ignore
use guest_macro::guest;

// Define your provider
struct MyProvider;

impl MyProvider {
    fn new() -> Self {
        Self
    }
}

// Generate guest infrastructure
guest!({
    owner: "acme-corp",
    provider: MyProvider,
    http: [
        "/health": get(HealthRequest, HealthResponse),
        "/api/items/{item_id}": get(GetItemRequest, GetItemResponse),
        "/api/items": post(CreateItemRequest with_body, CreateItemResponse),
    ],
    messaging: [
        "item-events.v1": ItemEventMessage,
    ]
});
```

## License

MIT OR Apache-2.0
