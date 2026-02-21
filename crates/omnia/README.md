# Omnia Wasm Runtime

The Omnia Wasm runtime provides a thin wrapper around [`wasmtime`](https://github.com/bytecodealliance/wasmtime) for ergonomic integration of host-based services for WASI components.

It allows you to declaratively assemble a runtime that provides specific capabilities (like HTTP, Key-Value, Messaging) to guest components, backed by real host implementations.

## Quick Start

Use the `runtime!` macro to configure which WASI interfaces and backends your host runtime needs. This generates a `runtime_run` function that handles the entire lifecycle.

```rust,ignore
use omnia::runtime;
use omnia_wasi_http::WasiHttpCtx;
use omnia_wasi_keyvalue::KeyValueDefault;
use omnia_wasi_otel::DefaultOtel;

// Define the runtime with required capabilities
omnia::runtime!({
    "http": WasiHttpCtx,
    "keyvalue": KeyValueDefault,
    "otel": DefaultOtel,
});

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    // Parse command line arguments (provided by the macro-generated Cli)
    let cli = Cli::parse();

    // Run the runtime
    runtime_run(cli).await
}
```

## Core Traits

The runtime is built around a set of traits that allow services to be plugged in:

| Trait | Purpose |
| ----- | ------- |
| `Host<T>` | Links a WASI interface (e.g., `wasi:http`) into the `wasmtime::Linker`. |
| `Server<S>` | Starts a server (e.g., HTTP listener, NATS subscriber) to handle incoming requests. |
| `Backend` | Connects to an external service (e.g., Redis, Postgres) during startup. |
| `State` | Manages per-request state and provides access to the component instance. |
| `FromEnv` | Configures backend connections from environment variables. |

## Features

- **`jit`** (default): Enables Cranelift JIT compilation, allowing you to run `.wasm` files directly. Disable this to only support pre-compiled `.bin` components (useful for faster startup in production).

## Configuration

The runtime and its included services are configured via environment variables:

- **`RUST_LOG`**: Controls logging verbosity (e.g., `info`, `debug`, `omnia=trace`).
- **`OTEL_GRPC_URL`**: Endpoint for OpenTelemetry collector (if `omnia-otel` is used).

## Architecture

See the [workspace documentation](https://github.com/augentic/omnia) for the full architecture guide and list of available WASI interface crates.

## License

MIT OR Apache-2.0
