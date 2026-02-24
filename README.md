# Omnia: Lightweight WebAssembly Runtime

Omnia is a lightweight, secure runtime for WebAssembly (WASI) components. It provides a thin, ergonomic wrapper around `[wasmtime](https://github.com/bytecodealliance/wasmtime)` to easily integrate host-based services like HTTP, messaging, and key-value stores into your WASI applications.

While it can be used standalone, Omnia is primarily designed to be the runtime for **Augentic's Agent Skills**. It ensures that agent-generated code runs in a safe, sandboxed environment while still having controlled access to necessary infrastructure.

## Why Omnia?

- **Secure by Default**: All guest code runs in a strict WebAssembly sandbox. Capabilities (network, filesystem) are explicitly granted.
- **Batteries Included**: Comes with built-in support for common WASI interfaces: HTTP, Key-Value, Messaging, SQL, and more.
- **Developer Friendly**: Provides a rich SDK (`omnia-sdk`) and macros (`guest!`, `runtime!`) to eliminate boilerplate.
- **Pluggable Architecture**: Easily swap out backend implementations (e.g., switch from in-memory to Redis for Key-Value) without changing guest code.

## Features

- **WASI 0.2 support**: Full support for the Component Model and WASI Preview 2.
- **Host Services**:
  - **HTTP**: Outbound requests and incoming server handling
  - **Key-Value**: Simple get/set/delete operations (default: in-memory)
  - **Messaging**: Pub/Sub patterns (default: in-memory broadcast)
  - **SQL**: Database access via ORM (default: SQLite)
  - **Observability**: OpenTelemetry tracing and metrics built-in
  - **Blobstore**: Supports blob storage and retrieval operations 
  - **Identity**: Basic access token provisioning for guest components
- **Macros**:
  - `runtime!`: Declaratively configure your host runtime.
  - `guest!`: Wire up guest handlers with minimal code.

## Examples

The `examples` directory contains complete working examples of guests and runtimes.

**[Explore the Examples](./examples/README.md)**

### Docker

To build a production-ready Docker image

#### Step One: Create a runtime project

Create a simple runtime project with support for HTTP, Key-Value, and OpenTelemetry.

```rust,ignore
// main.rs
use omnia_opentelemetry::Client as OpenTelemetry;
use omnia_redis::Client as Redis;
use omnia_wasi_http::{HttpDefault, WasiHttp};
use omnia_wasi_keyvalue::WasiKeyValue;
use omnia_wasi_otel::WasiOtel;

buildgen::runtime!(main, {
    WasiHttp: HttpDefault,
    WasiKeyValue: Redis,
    WasiOtel: OpenTelemetry,
});
```

#### Step Two: Build the Docker image

```bash
docker build --tag ghcr.io/augentic/omnia .
```

## Crates

| Crate                                           | Description                                                                |
| ----------------------------------------------- | -------------------------------------------------------------------------- |
| `[omnia](crates/omnia)`                         | Core runtime -- wasmtime wrapper with CLI and pluggable WASI host services |
| `[omnia-sdk](crates/omnia-sdk)`                 | Guest SDK -- traits, error types, and macros for WASI component authors    |
| `[omnia-orm](crates/orm)`                       | ORM layer for wasi-sql with fluent query builder                           |
| `[omnia-otel](crates/otel)`                     | OpenTelemetry tracing and metrics for the runtime                          |
| `[omnia-guest-macro](crates/guest-macro)`       | `guest!` proc-macro for guest HTTP/messaging handlers                      |
| `[omnia-runtime-macro](crates/runtime-macro)`   | `runtime!` proc-macro for host runtime generation                          |
| `[omnia-wasi-blobstore](crates/wasi-blobstore)` | wasi:blobstore host and guest bindings                                     |
| `[omnia-wasi-config](crates/wasi-config)`       | wasi:config host and guest bindings                                        |
| `[omnia-wasi-http](crates/wasi-http)`           | wasi:http host and guest bindings                                          |
| `[omnia-wasi-identity](crates/wasi-identity)`   | wasi:identity host and guest bindings                                      |
| `[omnia-wasi-keyvalue](crates/wasi-keyvalue)`   | wasi:keyvalue host and guest bindings                                      |
| `[omnia-wasi-messaging](crates/wasi-messaging)` | wasi:messaging host and guest bindings                                     |
| `[omnia-wasi-otel](crates/wasi-otel)`           | wasi:otel host and guest bindings                                          |
| `[omnia-wasi-otel-attr](crates/wasi-otel-attr)` | `#[instrument]` attribute macro for WASI otel                              |
| `[omnia-wasi-sql](crates/wasi-sql)`             | wasi:sql host and guest bindings                                           |
| `[omnia-wasi-vault](crates/wasi-vault)`         | wasi:vault host and guest bindings                                         |
| `[omnia-wasi-websocket](crates/wasi-websocket)` | wasi:websocket host and guest bindings                                     |


## License

MIT OR Apache-2.0