# omnia-host-macros

Procedural macros for generating host-side WebAssembly Component Runtime infrastructure.

## Overview

This crate provides the `runtime!` macro that generates the necessary runtime infrastructure for executing WebAssembly components with WASI capabilities. Instead of manually managing feature flags and conditional compilation, you declaratively specify which WASI interfaces and backends your runtime needs.

## Usage

Add `omnia` to your dependencies (the `runtime!` macro is re-exported from the `omnia` crate):

```toml
[dependencies]
omnia = { workspace = true }
```

Then use the `runtime!` macro to generate your runtime infrastructure:

```rust,ignore
use omnia::runtime;

// Import the backend types you want to use
use omnia_wasi_http::WasiHttpCtx;
use omnia_wasi_otel::DefaultOtel;
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
// - a `Backends` bundle: one connected backend per declared interface
// - the `HasXxx` accessor impls wiring each backend to the library's blanket views
// - a `main` entry point that delegates to `omnia::main`
```

## Configuration Format

The macro accepts a map-like syntax:

```rust,ignore
runtime!({
    "interface_name": BackendType,
    // ...
});
```

### Supported Interfaces

- **`http`**: HTTP client and server - Backend: `WasiHttpCtx` (marker type, no backend connection needed)

- **`otel`**: OpenTelemetry observability - Backend: `DefaultOtel` (connects to OTEL collector)

- **`blobstore`**: Object/blob storage - Backends: `MongoDb` or `Nats`

- **`keyvalue`**: Key-value storage - Backends: `Nats` or `Redis`

- **`messaging`**: Pub/sub messaging - Backends: `Nats` or `Kafka`

- **`vault`**: Secrets management - Backend: `Azure` (Azure Key Vault)

- **`sql`**: SQL database - Backend: `Postgres`

- **`identity`**: Identity and authentication - Backend: `Azure` (Azure Identity)

- **`websocket`**: WebSocket connections - Backend: `WebSocketCtxImpl` (default implementation for development use)

## Generated Code

The macro generates a private `runtime` module containing:

### `Backends` bundle

A `Clone` struct with one connected backend per declared `Host: Backend` wiring, plus its `omnia::Backends` impl whose `connect()` connects every backend concurrently. A deployment that declares no backends uses the library's `()` bundle, so nothing is generated.

```rust,ignore
#[derive(Clone)]
struct Backends {
    // ... one field per declared backend
}

impl omnia::Backends for Backends {
    // connect every backend concurrently
    async fn connect() -> Result<Self> { /* ... */ }
}
```

### WASI view accessor impls

For each declared interface, the macro emits the `HasXxx` accessor impl that exposes the bundle's backend to the library's blanket `WasiXxxView for omnia::StoreCtx<Backends>` impl. Most interfaces share one accessor shape; `wasi:http` and `wasi:config` use slightly different ones, handled as special cases in codegen.

### `main` entry point

A `#[tokio::main]` `main` that delegates to `omnia::main::<Backends, Hooks>`, where `Hooks` is a generated [`Wiring`] impl: [`Wiring::link`](omnia::Wiring::link) runs inside `omnia::Runtime::new` to link hosts, connect backends, and assemble the registry; [`Wiring::serve`](omnia::Wiring::serve) launches each trigger host's `run`. The host runtime is the library `omnia::Runtime<Backends>`; the macro no longer emits a runtime type or trait impl of its own.

## Example: Custom Initiator Configuration

You can create different runtime configurations for different use cases:

```rust,ignore
// Minimal HTTP server
mod http_runtime {
    use omnia_wasi_http::WasiHttpCtx;

    omnia::runtime!({
        "http": WasiHttpCtx
    });
}

// Full-featured runtime
mod full_runtime {
    use omnia_wasi_http::WasiHttpCtx;
    use omnia_wasi_otel::DefaultOtel;
    use be_nats::Client as Nats;

    omnia::runtime!({
        "http": WasiHttpCtx,
        "otel": DefaultOtel,
        "keyvalue": Nats,
        "messaging": Nats,
        "blobstore": Nats
    });
}
```

Now you can declaratively specify your configuration:

```rust,ignore
mod omnia_runtime {
    omnia::runtime!({
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
