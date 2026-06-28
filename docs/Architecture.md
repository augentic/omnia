# Architecture

This document describes the architecture of Omnia (WebAssembly Runtime), a modular WASI component runtime built on [wasmtime](https://github.com/bytecodealliance/wasmtime).

## Overview

Omnia provides a thin wrapper around wasmtime for ergonomic integration of host-based services for WASI components. It enables WebAssembly guests to interact with external services (databases, message queues, etc.) through standardized WASI interfaces, while allowing hosts to swap backend implementations without changing guest code.

```text
┌─────────────────────────────────────────────────────────────────────┐
│                           Host Runtime                              │
│                                                                     │
│  ┌────────────┐  ┌────────────┐  ┌────────────┐  ┌────────────┐    │
│  │  Backend   │  │  Backend   │  │  Backend   │  │  Backend   │    │
│  │  (Redis)   │  │  (Kafka)   │  │  (Azure)   │  │  (NATS)    │    │
│  └─────┬──────┘  └─────┬──────┘  └─────┬──────┘  └─────┬──────┘    │
│        │               │               │               │            │
│  ┌─────┴──────┐  ┌─────┴──────┐  ┌─────┴──────┐  ┌─────┴──────┐    │
│  │ wasi-kv    │  │ wasi-msg   │  │ wasi-vault │  │ wasi-blob  │    │
│  │ (WASI API) │  │ (WASI API) │  │ (WASI API) │  │ (WASI API) │    │
│  └─────┬──────┘  └─────┬──────┘  └─────┬──────┘  └─────┬──────┘    │
│        │               │               │               │            │
│        └───────────────┴───────┬───────┴───────────────┘            │
│                                │                                    │
│                         ┌──────┴──────┐                             │
│                         │   omnia    │                             │
│                         │ (wasmtime)  │                             │
│                         └──────┬──────┘                             │
│                                │                                    │
│   ┌────────────────────────────┴────────────────────────────────┐   │
│   │                     WebAssembly Guest                       │   │
│   │              (Your application logic - .wasm)               │   │
│   └─────────────────────────────────────────────────────────────┘   │
└─────────────────────────────────────────────────────────────────────┘
```

## Core Concepts

### Guest/Host Architecture

Omnia follows the WebAssembly Component Model's guest/host pattern:

- **Guest**: Application code compiled to WebAssembly (`.wasm`). Uses WASI interfaces to interact with the outside world. The guest is portable and backend-agnostic.

- **Host**: The native runtime that loads and executes the WebAssembly guest. Provides concrete implementations of WASI interfaces by connecting to actual backends (Redis, Kafka, Postgres, etc.).

This separation allows the same guest code to run with different backends—swap Redis for NATS without changing application logic.

### Three-Layer Architecture

Omnia is organized into three distinct layers:

```text
┌─────────────────────────────────────────────────────────────────┐
│  Layer 3: Backends (be-*)                                       │
│  Concrete connections to external services                      │
│  Examples: be-redis, be-kafka, be-nats, be-azure, be-postgres   │
├─────────────────────────────────────────────────────────────────┤
│  Layer 2: WASI Interfaces (wasi-*)                              │
│  Abstract service capabilities defined by WIT interfaces        │
│  Examples: wasi-keyvalue, wasi-messaging, wasi-blobstore        │
├─────────────────────────────────────────────────────────────────┤
│  Layer 1: Kernel                                                │
│  Core runtime infrastructure (wasmtime, CLI, traits)            │
└─────────────────────────────────────────────────────────────────┘
```

## Crate Organization

### Kernel (`crates/omnia`)

The foundation of the runtime. Provides:

- **CLI infrastructure**: Command-line interface for running and compiling WebAssembly components
- **Core traits**: `Runtime`, `Host`, `Server`, and `Backend` traits that all components implement
- **Wasmtime integration**: Re-exports and wrappers for wasmtime functionality

Key traits:

```rust
/// Implemented by all WASI hosts to link their dependencies
pub trait Host<T>: Debug + Sync + Send {
    fn add_to_linker(linker: &mut Linker<T>) -> Result<()>;
}

/// Implemented by WASI hosts that are servers
pub trait Server<R: Runtime>: Debug + Sync + Send {
    fn run(&self, state: &R) -> impl Future<Output = Result<()>>;
}

/// Implemented by backend resources for connection management
pub trait Backend: Sized + Sync + Send {
    type ConnectOptions: FromEnv;
    fn connect_with(options: Self::ConnectOptions) -> impl Future<Output = Result<Self>>;
}
```

### WASI Interface Crates (`crates/wasi-*`)

Each WASI interface crate provides both guest and host implementations:

| Crate            | WASI Interface   | Purpose                     |
| ---------------- | ---------------- | --------------------------- |
| `wasi-http`      | `wasi:http`      | HTTP client/server          |
| `wasi-keyvalue`  | `wasi:keyvalue`  | Key-value storage           |
| `wasi-messaging` | `wasi:messaging` | Pub/sub messaging           |
| `wasi-blobstore` | `wasi:blobstore` | Object/blob storage         |
| `wasi-sql`       | `wasi:sql`       | SQL database access         |
| `wasi-vault`     | Custom           | Secrets management          |
| `wasi-identity`  | Custom           | Identity/authentication     |
| `wasi-otel`      | Custom           | OpenTelemetry observability |
| `wasi-websocket` | Custom           | WebSocket connections       |

Each crate contains:

```text
wasi-keyvalue/
├── src/
│   ├── lib.rs          # Conditional compilation (guest vs host)
│   ├── guest.rs        # Guest-side bindings (wasm32)
│   └── host/           # Host-side implementation (native)
│       ├── mod.rs      # Service struct, Host/Server trait impls
│       ├── store_impl.rs
│       ├── default_impl.rs
│       └── resource.rs
└── wit/                # WIT interface definitions
    ├── world.wit
    └── deps/           # WASI standard definitions
```

The conditional compilation allows the same crate to be used by both guests (compiled to wasm32) and hosts (compiled to native):

```rust
#[cfg(target_arch = "wasm32")]
mod guest;
#[cfg(target_arch = "wasm32")]
pub use guest::*;

#[cfg(not(target_arch = "wasm32"))]
mod host;
#[cfg(not(target_arch = "wasm32"))]
pub use host::*;
```

### Backend Crates (`crates/be-*`)

Backend crates provide concrete implementations connecting to external services:

| Crate              | Service        | Supports                       |
| ------------------ | -------------- | ------------------------------ |
| `be-redis`         | Redis          | keyvalue                       |
| `be-nats`          | NATS           | keyvalue, messaging, blobstore |
| `be-kafka`         | Apache Kafka   | messaging                      |
| `be-mongodb`       | MongoDB        | blobstore                      |
| `be-postgres`      | PostgreSQL     | sql                            |
| `be-azure`         | Azure          | vault, identity                |
| `be-opentelemetry` | OTEL Collector | otel                           |

Each backend:

1. Implements the `Backend` trait for connection management
2. Implements the context trait for its supported WASI interfaces (e.g., `WasiKeyValueCtx`)
3. Loads configuration from environment variables via `FromEnv`

Example backend structure:

```rust
#[derive(Clone)]
pub struct Client(ConnectionManager);

impl Backend for Client {
    type ConnectOptions = ConnectOptions;

    async fn connect_with(options: Self::ConnectOptions) -> Result<Self> {
        // Connect to the service...
    }
}

// Implement WASI interface contexts
impl WasiKeyValueCtx for Client {
    fn open_bucket(&self, identifier: String) -> FutureResult<Arc<dyn Bucket>> {
        // Provide keyvalue functionality via Redis...
    }
}
```

### Build Generation (`crates/buildgen`)

The `buildgen` crate provides the `runtime!` macro that generates runtime infrastructure from a declarative configuration:

```rust
use buildgen::runtime;

runtime!({
    WasiHttp: HttpDefault,
    WasiOtel: OpenTelemetry,
    WasiKeyValue: Redis,
    WasiMessaging: Kafka,
    WasiVault: Azure,
});
```

The macro generates:

- `Context`: Holds the guest registry and backend connections
- `StoreCtx`: Per-instance data shared between runtime and host functions
- `Runtime` trait implementation (via `#[derive(Runtime)]`)
- WASI view trait implementations for each interface (via `#[derive(StoreContext)]`)
- `Context::new` to link hosts, connect backends, and assemble the registry
- `main` that delegates to `omnia::main` (CLI parse, compile, bootstrap, and `run`)

## WIT Interface Definitions

WASI interfaces are defined using [WIT (WebAssembly Interface Types)](https://component-model.bytecodealliance.org/design/wit.html). Each `wasi-*` crate contains a `wit/` directory with interface definitions:

```wit
// wasi-keyvalue/wit/world.wit
package wasi:keyvalue;

world keyvalue {
    include wasi:keyvalue/imports@0.2.0-draft2;
}
```

Dependencies on standard WASI definitions are managed in `wit/deps/` and versioned via `deps.toml`.

## Runtime Execution Flow

1. **CLI parsing**: Generated `main` delegates to `omnia::main`, which parses the `run` subcommand (or `compile` when the `jit` feature is enabled)

2. **Compile**: `RegistryBuilder` loads the manifest or wasm, compiles guests, and returns a `Compiled` registry plan

3. **Bootstrap**: `Context::new` links WASI hosts, connects backends, and builds the `Registry`

4. **Drive**: `run` either invokes `command::run` (one-shot `wasi:cli`) or `prepare`s the runtime and awaits every long-lived trigger server to completion

5. **Request handling** (server mode): Trigger hosts (`WasiHttp`, `WasiMessaging`, `WasiWebSocket`) accept requests, instantiate guests per call, and return responses

```text
CLI → Compile → Context::new → run
                                 ├─ command mode → command::run → ExitStatus
                                 └─ server mode  → prepare → trigger servers
```

## Configuration

All backends use environment variables for configuration. The `FromEnv` derive macro (from the `fromenv` crate) provides automatic parsing:

```rust
#[derive(Debug, Clone, FromEnv)]
pub struct ConnectOptions {
    #[env(from = "REDIS_URL", default = "redis://localhost:6379")]
    pub url: String,
    #[env(from = "REDIS_MAX_RETRIES", default = "3")]
    pub max_retries: usize,
}
```

See individual backend READMEs for specific environment variables.

## Directory Structure

```text
omnia/
├── src/                    # CLI entry points (omnia binaries)
├── crates/
│   ├── omnia/             # Core runtime infrastructure
│   ├── buildgen/           # Runtime code generation macro
│   ├── wasi-*/             # WASI interface implementations
│   │   ├── src/
│   │   │   ├── guest.rs    # Guest bindings (wasm32)
│   │   │   └── host/       # Host implementation (native)
│   │   └── wit/            # WIT interface definitions
│   └── be-*/               # Backend implementations
├── examples/               # Example guests and hosts
│   └── <example>/
│       ├── lib.rs          # Guest code (→ .wasm)
│       └── main.rs         # Host code (→ native binary)
├── docker/                 # Docker compose files for services
└── scripts/                # Helper scripts for development
```

## Adding a New WASI Interface

1. Create `crates/wasi-<name>/` with the standard structure
2. Define the WIT interface in `wit/world.wit`
3. Implement guest bindings in `src/guest.rs`
4. Implement host functionality in `src/host/`
5. Export the `Host` trait implementation
6. Update `buildgen` to support the new interface
7. Create example(s) in `examples/`

## Adding a New Backend

1. Create `crates/be-<name>/`
2. Implement the `Backend` trait for connection management
3. Implement context traits for supported WASI interfaces (e.g., `WasiKeyValueCtx`)
4. Add `ConnectOptions` with `FromEnv` derive
5. Update `buildgen` to support the new backend type
6. Create example(s) demonstrating the backend

## Related Documentation

- [wasmtime Component Model](https://docs.wasmtime.dev/api/wasmtime/component/)
- [WASI Proposals](https://github.com/WebAssembly/WASI/blob/main/Proposals.md)
- [WIT Format](https://component-model.bytecodealliance.org/design/wit.html)
- [examples/README.md](./examples/README.md) - Running examples
