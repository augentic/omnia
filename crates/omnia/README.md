# Omnia Wasm Runtime

The Omnia Wasm runtime provides a thin wrapper around [`wasmtime`](https://github.com/bytecodealliance/wasmtime) for ergonomic integration of host-based services for WASI components.

It allows you to declaratively assemble a runtime that provides specific capabilities (like HTTP, Key-Value, Messaging) to guest components, backed by real host implementations.

## Quick Start

Use the `runtime!` macro to declare which WASI interfaces (`hosts`) and backends your host runtime needs. It generates the `StoreCtx`, the `Runtime` implementation, the WASI links, the trigger servers, and a `main` that parses the CLI and serves.

```rust,ignore
use omnia_wasi_http::{HttpDefault, WasiHttp};
use omnia_wasi_otel::{OtelDefault, WasiOtel};

omnia::runtime!({
    hosts: {
        WasiHttp: HttpDefault,
        WasiOtel: OtelDefault,
    }
});
```

Each `Host: Backend` pair links a WASI interface and binds it to a host backend. The macro always generates the `main` entry point; for a custom entry point, hand-write the runtime instead (derive `Runtime`/`StoreContext` and call `omnia::serve`).

## Core Traits

The runtime is built around a set of traits that allow services to be plugged in:

| Trait       | Purpose                                                                             |
| ----------- | ----------------------------------------------------------------------------------- |
| `Host<T>`   | Links a WASI interface (e.g., `wasi:http`) into the `wasmtime::Linker`.             |
| `Server<R>` | Starts a server (e.g., HTTP listener, NATS subscriber) to handle incoming requests. |
| `Backend`   | Connects to an external service (e.g., Redis, Postgres) during startup.             |
| `Runtime`   | Manages per-request state and provides access to the component instance.            |
| `FromEnv`   | Configures backend connections from environment variables.                          |

## Public API

`omnia` exposes only what a deployment author, a host-server crate, or a hand-written runtime needs; lifecycle, dispatch, manifest, and transport-carrier internals are crate-private.

- **Macros:** `runtime!`, `#[derive(Runtime)]`, `#[derive(StoreContext)]`
- **Lifecycle:** `serve` (long-lived triggers) and `run_command` (one-shot `wasi:cli` command) — both drive epoch interruption, pool-metric sampling, and host-mediated link serving
- **Runtime + store:** `Runtime`, `StoreBase`, `Host`, `Server`, `Backend`, `FromEnv`, `HasLimits`, `HostDispatch`, `FutureResult`
- **Registry pipeline:** `RegistryBuilder`, `Compiled`, `Registry`, `Guest`, `GuestId`, `RuntimeOptions`
- **Trigger routing (host servers):** `HttpRoutes`, `TopicRoutes`, `Routes`, `Resolver`, `TriggerRouter`
- **Host-mediated linking (advanced):** `serve_links`, `GuestSelector`, `FirstArgSelector`, `LinkClient`, `WrpcState`
- **Telemetry + CLI:** `Telemetry`, `resource`, `Cli`, `Command`, `Parser`

Most deployments only touch the `runtime!` macro; a hand-written runtime instead derives `Runtime`/`StoreContext` (or implements them) and calls `serve`.

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
