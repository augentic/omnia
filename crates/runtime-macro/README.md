# buildgen

Procedural macros for generating WebAssembly Component Initiator infrastructure.

## Overview

This crate provides the `runtime!` macro that generates the necessary runtime infrastructure for executing WebAssembly components with WASI capabilities. Instead of manually managing feature flags and conditional compilation, you declaratively specify which WASI interfaces and backends your runtime needs.

## Usage

Add `buildgen` to your dependencies:

```toml
[dependencies]
buildgen = { workspace = true }
```

Then use the `runtime!` macro to generate your runtime infrastructure:

```rust
use buildgen::runtime;

// Import the backend types you want to use
use wasi_http::WasiHttpCtx;
use wasi_otel::DefaultOtel;
use be_mongodb::Client as MongoDb;
use be_nats::Client as Nats;
use be_azure::Client as Azure;

// Generate runtime infrastructure
runtime!({
    "http": WasiHttpCtx,
    "otel": DefaultOtel,
    "blobstore": MongoDb,
    "keyvalue": Nats,
    "messaging": Nats,
    "vault": Azure
});

// The macro generates:
// - RuntimeContext struct with backend connections
// - RuntimeStoreCtx struct with per-instance contexts
// - State trait implementation
// - WASI view trait implementations
// - runtime_run() function
```

## Configuration Format

The macro accepts a map-like syntax:

```rust
runtime!({
    "interface_name": BackendType,
    // ...
});
```

### Supported Interfaces

- **`http`**: HTTP client and server
  - Backend: `WasiHttpCtx` (marker type, no backend connection needed)

- **`otel`**: OpenTelemetry observability
  - Backend: `DefaultOtel` (connects to OTEL collector)

- **`blobstore`**: Object/blob storage
  - Backends: `MongoDb` or `Nats`

- **`keyvalue`**: Key-value storage
  - Backends: `Nats` or `Redis`

- **`messaging`**: Pub/sub messaging
  - Backends: `Nats` or `Kafka`

- **`vault`**: Secrets management
  - Backend: `Azure` (Azure Key Vault)

- **`sql`**: SQL database
  - Backend: `Postgres`

- **`identity`**: Identity and authentication
  - Backend: `Azure` (Azure Identity)

- **`websockets`**: WebSocket connections
  - Backend: `WebSocketsCtxImpl` (default implementation for development use)

## Generated Code

The macro generates the following:

### RuntimeContext

A struct holding pre-instantiated components and backend connections:

```rust
#[derive(Clone)]
struct RuntimeContext {
    instance_pre: InstancePre<RuntimeStoreCtx>,
    // ... backend fields
}
```

### RuntimeStoreCtx

Per-instance data shared between the WebAssembly runtime and host functions:

```rust
pub struct RuntimeStoreCtx {
    pub table: ResourceTable,
    pub wasi: WasiCtx,
    // ... interface context fields
}
```

### State Trait Implementation

Implements the `State` trait from the `runtime` crate, providing methods to create new store contexts and access the pre-instantiated component.

### WASI View Implementations

Implements view traits for each configured WASI interface, allowing the WebAssembly guest to call host functions.

### runtime_run() Function

A public async function that:

1. Loads runtime configuration
2. Compiles the WebAssembly component
3. Links WASI interfaces
4. Connects to backends
5. Starts server interfaces (HTTP, messaging, WebSockets)

## Example: Custom Initiator Configuration

You can create different runtime configurations for different use cases:

```rust
// Minimal HTTP server
mod http_runtime {
    use wasi_http::WasiHttpCtx;

    yetti::runtime!({
        "http": WasiHttpCtx
    });
}

// Full-featured runtime
mod full_runtime {
    use wasi_http::WasiHttpCtx;
    use wasi_otel::DefaultOtel;
    use be_nats::Client as Nats;

    yetti::runtime!({
        "http": WasiHttpCtx,
        "otel": DefaultOtel,
        "keyvalue": Nats,
        "messaging": Nats,
        "blobstore": Nats
    });
}
```

Now you can declaratively specify your configuration:

```rust
mod credibil_runtime {
    yetti::runtime!({
        "http": WasiHttpCtx,
        "otel": DefaultOtel,
        "blobstore": MongoDb,
        "keyvalue": Nats,
        "messaging": Nats,
        "vault": Azure
    });
}
```

This provides:

- **Better readability**: The configuration is explicit and self-documenting
- **Less boilerplate**: No need for complex feature flag combinations
- **Type safety**: Backend types are checked at compile time
- **Flexibility**: Easy to create multiple runtime configurations in the same binary

## License

MIT OR Apache-2.0
