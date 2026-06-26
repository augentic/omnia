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

| Trait       | Purpose                                                                             |
| ----------- | ----------------------------------------------------------------------------------- |
| `Host<T>`   | Links a WASI interface (e.g., `wasi:http`) into the `wasmtime::Linker`.             |
| `Server<R>` | Starts a server (e.g., HTTP listener, NATS subscriber) to handle incoming requests. |
| `Backend`   | Connects to an external service (e.g., Redis, Postgres) during startup.             |
| `Runtime`   | Manages per-request state and provides access to the component instance.            |
| `FromEnv`   | Configures backend connections from environment variables.                          |

## Features

- **`jit`** (default): Enables Cranelift JIT compilation, allowing you to run `.wasm` files directly. Disable this to only support pre-compiled `.bin` components (useful for faster startup in production).

## Configuration

The runtime and its included services are configured via environment variables:

- **`RUST_LOG`**: Controls logging verbosity (e.g., `info`, `debug`, `omnia=trace`).
- **`OTEL_GRPC_URL`**: Endpoint for the OpenTelemetry collector used to export traces and metrics.

## Telemetry

The runtime reports OpenTelemetry tracing and metrics out-of-the-box. During startup, `omnia` configures `tracing-subscriber`, OTLP span exporters, and metric readers via the `Telemetry` builder, so host runtimes emit telemetry without extra wiring. Most applications never need to call this directly:

```rust,ignore
use omnia::Telemetry;

// Minimal -- uses RUST_LOG for filtering, no OTLP export
Telemetry::new("my-service").build()?;

// With OTLP export to a collector
Telemetry::new("my-service")
    .endpoint("http://localhost:4317")
    .build()?;
```

The `OTEL_GRPC_URL` environment variable is respected if no explicit endpoint is set.

## Architecture

See the [workspace documentation](https://github.com/augentic/omnia) for the full architecture guide and list of available WASI interface crates.

## License

MIT OR Apache-2.0
