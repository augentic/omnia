# Omnia Wasm Runtime

The Omnia Wasm runtime provides a thin wrapper around [`wasmtime`](https://github.com/bytecodealliance/wasmtime) for ergonomic integration of host-based services for WASI components.

We consider this a stop-gap solution until production-grade runtimes support dynamic inclusion of host-based services.

## Quick Start

Use the `runtime!` macro to declaratively configure which WASI interfaces and backends your host runtime needs:

```rust,ignore
use omnia::runtime;
use omnia_wasi_http::WasiHttpCtx;
use omnia_wasi_otel::DefaultOtel;

omnia::runtime!({
    "http": WasiHttpCtx,
    "otel": DefaultOtel,
});
```

The macro generates a `runtime_run()` async function that compiles the WebAssembly component, connects backends, links WASI interfaces, and starts server loops.

## Core Traits

| Trait | Purpose |
|-------|---------|
| `Host<T>` | Link a WASI interface into the wasmtime `Linker` |
| `Server<S>` | Start a server (HTTP, messaging, WebSocket) for incoming requests |
| `Backend` | Connect to an external service (Redis, NATS, Postgres, etc.) |
| `State` | Create per-request store contexts and access the pre-instantiated component |
| `FromEnv` | Build backend connection options from environment variables |

## Features

- **`jit`** (default) -- enables Cranelift JIT compilation so you can load `.wasm` files directly. Without it only pre-compiled `.bin` files are accepted.

## Architecture

See the [workspace documentation](https://github.com/augentic/omnia) for the full architecture guide and list of available WASI interface crates.

## License

MIT OR Apache-2.0
