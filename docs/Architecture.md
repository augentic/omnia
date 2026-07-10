# Architecture

This document explains how Omnia is put together: the layering, the core abstractions, and the execution flow from CLI to guest invocation. It is background reading — for hands-on material, start with [Getting Started](getting-started.md) and the [how-to guides](README.md#how-to-guides).

For shared terminology (**runtime core**, **host-injected tools**, **Law 2**, and when "floor" means something else), see the [Glossary](glossary.md).

## Overview

Omnia is a thin, opinionated wrapper around [wasmtime](https://github.com/bytecodealliance/wasmtime) for running WASI components. It lets WebAssembly guests interact with external services (databases, message queues, models, and so on) through standardized WASI interfaces, while hosts swap backend implementations without changing guest code.

```text
┌─────────────────────────────────────────────────────────────────────┐
│                           Host Runtime                              │
│                                                                     │
│  ┌────────────┐  ┌────────────┐  ┌────────────┐  ┌────────────┐     │
│  │  Backend   │  │  Backend   │  │  Backend   │  │  Backend   │     │
│  │  (Redis)   │  │  (Kafka)   │  │  (genai)   │  │ (in-memory)│     │
│  └─────┬──────┘  └─────┬──────┘  └─────┬──────┘  └─────┬──────┘     │
│        │               │               │               │            │
│  ┌─────┴──────┐  ┌─────┴──────┐  ┌─────┴──────┐  ┌─────┴──────┐     │
│  │ wasi-kv    │  │ wasi-msg   │  │ wasi-model │  │ wasi-blob  │     │
│  │ (WASI API) │  │ (WASI API) │  │ (WASI API) │  │ (WASI API) │     │
│  └─────┬──────┘  └─────┬──────┘  └─────┬──────┘  └─────┬──────┘     │
│        │               │               │               │            │
│        └───────────────┴───────┬───────┴───────────────┘            │
│                                │                                    │
│                         ┌──────┴──────┐                             │
│                         │    omnia    │                             │
│                         │ (wasmtime)  │                             │
│                         └──────┬──────┘                             │
│                                │                                    │
│   ┌────────────────────────────┴─────────────────────────────────┐  │
│   │                    WebAssembly Guests                        │  │
│   │        (Your application logic — one or many .wasm)          │  │
│   └──────────────────────────────────────────────────────────────┘  │
└─────────────────────────────────────────────────────────────────────┘
```

## Core Concepts

### Guest/Host Architecture

Omnia follows the WebAssembly Component Model's guest/host pattern:

- **Guest**: Application code compiled to WebAssembly (`.wasm`), targeting `wasm32-wasip2` and using WASI Preview 3 bindings. The guest is portable and backend-agnostic.
- **Host**: The native runtime that loads and executes guests. It provides concrete implementations of WASI interfaces by connecting to actual backends.

This separation allows the same guest to run with different backends — swap the in-memory key-value store for Redis without changing application logic.

### Three-Layer Architecture

```text
┌─────────────────────────────────────────────────────────────────┐
│  Layer 3: Backends                                              │
│  Concrete connections to external services                      │
│  In-tree defaults (KeyValueDefault, SqlDefault, ...) and the    │
│  production crates in the backends repo (redis, kafka, ...)     │
├─────────────────────────────────────────────────────────────────┤
│  Layer 2: WASI Interfaces (crates/wasi-*)                       │
│  Abstract service capabilities defined by WIT interfaces        │
│  Examples: wasi-keyvalue, wasi-messaging, wasi-model            │
├─────────────────────────────────────────────────────────────────┤
│  Layer 1: Runtime core (crates/omnia)                           │
│  wasmtime engine, CLI, deployment/registry, dispatch, traits    │
└─────────────────────────────────────────────────────────────────┘
```

Layers 1 and 2 form the **runtime core** — domain-agnostic infrastructure that routes opaque identities and satisfies typed effects. Which backend serves an interface is deployment configuration the core never parses (the glossary's **Law 2**).

## Crate Organization

### Runtime core (`crates/omnia`)

The foundation of the runtime. Provides:

- **CLI infrastructure**: the `run` subcommand (and `compile`, with the `jit` feature)
- **Deployment pipeline**: `DeploymentBuilder` loads a single `.wasm` or a manifest, `Registry` holds pre-instantiated guests
- **Core traits**: `Host`, `Server`, `Backend`, `Wiring`, plus the concrete `Runtime<B>` over `StoreCtx<B>`
- **Host-mediated dispatch**: guest-to-guest linking over an in-process wRPC carrier
- **Telemetry**: `tracing` + OpenTelemetry bootstrap

Key traits:

```rust
/// Implemented by all WASI hosts to link their functions into the shared linker.
pub trait Host<T>: Debug + Sync + Send {
    fn add_to_linker(linker: &mut Linker<T>) -> Result<()>;
}

/// Implemented by WASI hosts that are trigger servers (HTTP, messaging, WebSocket).
pub trait Server<B>: Debug + Sync + Send {
    const IS_SERVER: bool = false;
    fn run(&self, state: &Runtime<B>) -> impl Future<Output = Result<()>>;
}

/// Implemented by backends for connection management.
pub trait Backend: Sized + Sync + Send {
    type ConnectOptions: FromEnv;
    fn connect() -> impl Future<Output = Result<Self>>;   // from environment
    fn connect_with(options: Self::ConnectOptions) -> impl Future<Output = Result<Self>>;
}
```

### WASI Interface Crates (`crates/wasi-*`)

Each interface crate provides guest bindings, a host implementation, and a default backend:

| Crate | Interface | Purpose | Default backend |
| ----- | --------- | ------- | --------------- |
| `wasi-http` | `wasi:http` | HTTP client/server (trigger) | `HttpDefault` — hyper client, axum server |
| `wasi-keyvalue` | `wasi:keyvalue` | Key-value storage | `KeyValueDefault` — in-memory cache |
| `wasi-messaging` | `wasi:messaging` | Pub/sub messaging (trigger) | `MessagingDefault` — in-process broadcast |
| `wasi-blobstore` | `wasi:blobstore` | Object/blob storage | `BlobstoreDefault` — in-memory |
| `wasi-sql` | `wasi:sql` | SQL access + guest ORM | `SqlDefault` — SQLite |
| `wasi-docstore` | Custom | JSON document store with filters | `DocStoreDefault` — embedded PoloDB |
| `wasi-config` | `wasi:config` | Runtime configuration | `ConfigDefault` — process environment |
| `wasi-vault` | Custom | Secrets management | `VaultDefault` — in-memory |
| `wasi-identity` | Custom | Identity/OAuth tokens | `IdentityDefault` — OAuth2 client flow |
| `wasi-otel` | Custom | Guest OpenTelemetry export | `OtelDefault` — log-only |
| `wasi-websocket` | Custom | WebSocket connections (trigger) | `WebSocketDefault` — tungstenite server |
| `wasi-model` | `omnia:model` | Model completions with grants | `ModelDefault` — fixture replay |

Conditional compilation lets one crate serve both sides — guests get bindings on `wasm32`, hosts get the implementation on native:

```rust
#[cfg(target_arch = "wasm32")]
mod guest;
#[cfg(not(target_arch = "wasm32"))]
mod host;
```

Each crate's `wit/` directory holds the [WIT](https://component-model.bytecodealliance.org/design/wit.html) interface definitions, with standard WASI dependencies vendored under `wit/deps/`.

### Guest SDK (`crates/omnia-guest`, `crates/guest-macros`)

`omnia-guest` defines transport-neutral `Operation`, `Invocation`, and provider-owning `Invoker` primitives plus explicit command, HTTP, and exact-topic messaging routers under `omnia_guest::api`. Applications own their WASI exports and call the matching transport adapter. The command router provides typed nested grammars, Clap-compatible help and completions, and exact output/exit projection. HTTP-aware errors (`HttpResult`), ORM query builders, and the MCP server module (`mcp::McpServer`, `mcp::router`) remain in `omnia-guest`; `guest-macros` provides only the independent `#[instrument]` tracing attribute.

### Host macro (`crates/host-macros`)

Provides the `runtime!` macro, re-exported as `omnia::runtime!`:

```rust
omnia::runtime!({
    mode: server,            // or `command`; server is the default
    hosts: {
        WasiHttp: HttpDefault,
        WasiOtel: OtelDefault,
        WasiKeyValue: Redis,
    }
});
```

The macro generates a `Backends` bundle (one connected backend per `Host: Backend` pair, with the accessor impls that wire each backend into the library's WASI views), a `Wiring` implementation whose `link` runs inside `Runtime::new` and whose `serve` launches each trigger server, and a `main` that delegates to `omnia::main`. The runtime itself is always the library type `omnia::Runtime<Backends>` over `omnia::StoreCtx<Backends>` — the macro emits wiring, not a runtime.

### Test scaffolding (`crates/testkit`)

Dev-only helpers for integration ("seam") tests: `find_guest` locates or builds an example guest `.wasm`, `temp_manifest` writes an ephemeral `omnia.toml`, and an in-process HTTP driver exercises HTTP guests without a network socket. See [the testing guide](guides/testing.md) for the testing policy.

## The Guest Registry

A deployment can hold many guests. All of them share one wasmtime `Engine` and one `Linker`; the `Registry` maps each opaque `GuestId` to a pre-instantiated `InstancePre`, so per-request instantiation is cheap. Three things hang off the registry:

- **Route tables** — per-trigger routing (`[[route.http]]` by longest prefix, `[[route.messaging]]`/`[[route.websocket]]` by NATS-style pattern) selects which guest handles an inbound request.
- **Mounts** — `[[mount]]` entries preopen host directories into every guest sandbox (read-only unless marked writable).
- **Dispatch** — per-guest `link` allow-lists name interfaces the host polyfills onto the shared linker; calls dispatch to whichever guest exports the interface, over an in-process carrier, with nesting bounded by `MAX_DISPATCH_DEPTH`.

All of this is declared in the `omnia.toml` manifest ([reference](reference/configuration.md#deployment-manifest-omniatoml)); a bare `.wasm` path on the command line remains the zero-config single-guest case.

## Runtime Execution Flow

1. **CLI parsing** — the generated `main` delegates to `omnia::main`, which parses the `run` subcommand.
2. **Build** — `DeploymentBuilder` loads the manifest or wasm, resolves mounts, compiles guests, and returns a `Deployment` ready for host linking.
3. **Assemble** — `Runtime::new` runs `Wiring::link` (each host's `add_to_linker`), connects backends (`Backends::connect`), and builds the `Registry` (pre-instantiating every guest).
4. **Bootstrap** — starts epoch interruption and pool-metric sampling, wires host-mediated link servers, then logs **`omnia ready`**.
5. **Drive** — command mode invokes the guest's `wasi:cli/run` once and exits with its status; server mode awaits every trigger server.
6. **Request handling** (server mode) — trigger hosts (`WasiHttp`, `WasiMessaging`, `WasiWebSocket`) accept requests, route to a guest, instantiate it in a fresh store, and return the response.

```text
CLI → Build → Runtime::new → bootstrap → run
                                            ├─ command mode → wasi:cli/run → ExitStatus
                                            └─ server mode  → trigger servers → per-request instantiate
```

### Isolation and pooling

Every invocation gets a **fresh instance in its own store** — no state survives between requests, and guests cannot observe each other except through host-mediated dispatch. To keep this cheap, the pooling instance allocator (on by default, `POOLING=true`) recycles instance slots; guest resource ceilings (`GUEST_TIMEOUT_MS`, `MAX_MEMORY_BYTES`, `MAX_FUEL`) bound each invocation. See [Configuration](reference/configuration.md) for the tunables.

## Configuration

All backends and runtime options use environment variables, parsed via the `FromEnv` derive:

```rust
#[derive(Debug, Clone, FromEnv)]
pub struct ConnectOptions {
    #[env(from = "REDIS_URL", default = "redis://localhost:6379")]
    pub url: String,
}
```

The consolidated list is in [Configuration](reference/configuration.md); individual backend READMEs document service-specific variables.

## Directory Structure

```text
omnia/
├── crates/
│   ├── omnia/              # Runtime core (engine, CLI, deployment, registry, dispatch)
│   ├── omnia-guest/        # Guest SDK (typed transport routers, errors, ORM, MCP)
│   ├── guest-macros/       # #[instrument] proc macro
│   ├── host-macros/        # runtime! proc-macro
│   ├── testkit/            # Integration-test scaffolding (dev-only)
│   └── wasi-*/             # WASI interface implementations
│       ├── src/
│       │   ├── guest.rs    # Guest bindings (wasm32)
│       │   └── host/       # Host implementation + default backend (native)
│       └── wit/            # WIT interface definitions
├── examples/               # One guest + runtime pair per capability
│   └── <example>/
│       ├── guest.rs        # Guest code (→ .wasm)
│       └── runtime.rs      # Host code (→ native binary)
└── docs/                   # This documentation
```

Production backends live in the sibling [`backends`](https://github.com/augentic/backends) repository, one crate per service, each implementing `Backend` plus the relevant `WasiXxxCtx` traits.

## Extending Omnia

**Adding a WASI interface**: create `crates/wasi-<name>/` with the standard layout, define the WIT world in `wit/`, implement guest bindings in `src/guest.rs` and the host (including a zero-config default backend) in `src/host/`, add a seam test in `tests/seam.rs`, and create an example.

**Adding a backend**: create a crate (usually in the `backends` repo), implement `Backend` with a `FromEnv`-derived `ConnectOptions`, implement the `WasiXxxCtx` trait(s) for the interfaces it serves, and add `#[ignore]`-gated live tests. No runtime-core changes are required — backends plug in through the `runtime!` host map.

## Related Documentation

- [Getting Started](getting-started.md) — first build and run
- [Configuration reference](reference/configuration.md) — env vars and manifest format
- [wasmtime Component Model](https://docs.wasmtime.dev/api/wasmtime/component/)
- [WIT Format](https://component-model.bytecodealliance.org/design/wit.html)
- [examples/README.md](../examples/README.md) — running the examples
