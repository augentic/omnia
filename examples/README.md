# Examples

Every WASI capability has a runnable example here. Each example is a **guest** (a WASI component compiled to `.wasm`, holding the application logic) plus a **runtime** (a native binary that loads the guest and provides its capabilities).

## Quick start

From the repo root, build and run any single-guest example with:

```bash
make build http
make run http
```

Or use Cargo directly — the two-step pattern behind the Makefile:

```bash
cargo build --example http-wasm --target wasm32-wasip2
cargo run --example http -- run ./target/wasm32-wasip2/debug/examples/http_wasm.wasm
```

(Guest artifact names use underscores: `http_wasm.wasm`, not `http-wasm.wasm`.)

Host startup logs (`initializing runtime`, trigger servers listening, and so on) use `tracing` at the `info` level. `make run` sets a sensible default `RUST_LOG`; override it when you need more detail. Without logging configured, the process stays quiet apart from Cargo's `Running …` line.

Each example directory has a `README.md` with test commands and example-specific setup.

## Example index

### Getting started

| Example | Demonstrates |
| ------- | ------------ |
| [`http`](http) | Basic HTTP server: WASI HTTP handler + Axum routing |
| [`keyvalue`](keyvalue) | Storing and retrieving state through `wasi:keyvalue` |
| [`cli`](cli) | A `wasi:cli/run` command-mode guest driven as a one-shot trigger |

### One per capability

| Example | Demonstrates |
| ------- | ------------ |
| [`blobstore`](blobstore) | Object storage containers and blobs (`wasi:blobstore`) |
| [`config`](config) | Reading runtime configuration (`wasi:config`) |
| [`docstore`](docstore) | JSON documents with filters, sorting, and pagination (`wasi:docstore`) |
| [`identity`](identity) | OAuth token acquisition (`wasi:identity`) |
| [`messaging`](messaging) | Pub-sub, request-reply, and fan-out (`wasi:messaging`) |
| [`model`](model) | Model completion across the `omnia:model` boundary with the scripted test double |
| [`otel`](otel) | Guest OpenTelemetry instrumentation (`wasi:otel`) |
| [`sql`](sql) | CRUD + the guest ORM, including a JOIN endpoint (`wasi:sql`) |
| [`vault`](vault) | Secret storage (`wasi:vault`) |
| [`websocket`](websocket) | Real-time bidirectional messaging (`wasi:websocket`) |

### Composition and deployment patterns

| Example | Demonstrates |
| ------- | ------------ |
| [`http-proxy`](http-proxy) | Outbound HTTP from a guest, with a keyvalue caching layer |
| [`http-routing`](http-routing) | Two HTTP guests behind path prefixes via a deployment manifest |
| [`guest-api`](guest-api) | Transport-neutral `Handler` implementations with typed HTTP routing |
| [`guest-link`](guest-link) | Host-mediated guest-to-guest linking over in-process wRPC |
| [`cli-static`](cli-static) | A compiled-in command deployment: inline guests, direct-command argv |
| [`mcp`](mcp) | A guest serving MCP tools and resources to AI agents over HTTP |

### Infrastructure

| Example | Demonstrates |
| ------- | ------------ |
| [`bench`](bench) | Self-contained HTTP load-test harness for pooling/latency tuning |

## Backends

All examples run against **in-memory** default backends — no external infrastructure needed. Data is process-local: keyvalue state is lost when the runtime stops, messages are delivered only within the process, SQL uses SQLite.

Swapping a default for a production backend (Redis, Kafka, Postgres, Azure, ...) is a one-line change in the runtime, with the guest untouched — see [Production Backends](../docs/guides/production-backends.md) for the wiring recipe:

```rust
omnia::runtime!({
    hosts: {
        WasiHttp: HttpDefault,
        WasiKeyValue: Redis,     // was: KeyValueDefault
    }
});
```

Some demos bind those production backends directly and live in the [`omnia-backends`](https://github.com/augentic/omnia-backends) repo — for example the [`cursor`](https://github.com/augentic/omnia-backends/tree/main/examples/cursor) end-to-end demo, whose guest calls `omnia-cursor` for a live completion while serving its own MCP docs tools over HTTP (the same pattern as the in-tree [`mcp`](mcp) guest). These need extra setup: credentials, CLI tools, or network access.

For a complete application-scale service built on these patterns, see [`omnia-exemplar`](https://github.com/augentic/omnia-exemplar).
