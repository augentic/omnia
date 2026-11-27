# Examples

Every WASI capability has a runnable example here. Each example is a **guest** (a WASI component compiled to `.wasm`, holding the application logic) plus a **runtime** (a native binary that loads the guest and provides its capabilities).

## Quick start

From the repo root, build and run any single-guest example with:

```bash
make example http
```

Or use Cargo directly — the two steps behind `make example`:

```bash
cargo build --example http-wasm --target wasm32-wasip3
cargo run --example http -- run ./target/wasm32-wasip3/debug/examples/http_wasm.wasm
```

(Guest artifact names use underscores: `http_wasm.wasm`, not `http-wasm.wasm`.)

Servers are quiet by default: `-v` shows startup and readiness, `-vv` each request. `make example` passes `-v`. `RUST_LOG` targets a single crate on top (`RUST_LOG=omnia_wasi_http=trace`); see [Verbosity flags](../docs/reference/configuration.md#verbosity-flags).

Each example directory has a `README.md` with test commands and example-specific setup.

## Example index

### Getting started

| Example | Demonstrates |
| ------- | ------------ |
| [`http`](http) | Basic HTTP server: WASI HTTP handler + Axum routing |
| [`keyvalue`](keyvalue) | Storing and retrieving state through `omnia:keyvalue` |
| [`cli`](cli) | A `wasi:cli/run` command-mode guest driven as a one-shot trigger |

### One per capability

| Example | Demonstrates |
| ------- | ------------ |
| [`blobstore`](blobstore) | Object storage containers and blobs (`omnia:blobstore`) |
| [`config`](config) | Reading runtime configuration (`omnia:config`) |
| [`docstore`](docstore) | JSON documents with filters, sorting, and pagination (`omnia:docstore`) |
| [`identity`](identity) | OAuth token acquisition (`omnia:identity`) |
| [`messaging`](messaging) | Pub-sub, request-reply, and fan-out (`omnia:messaging`) |
| [`model`](model) | Model completion across the `omnia:model` boundary with the scripted test double |
| [`otel`](otel) | Guest OpenTelemetry instrumentation (`omnia:otel`) |
| [`sql`](sql) | CRUD over `omnia:sql` through the `Database` helpers, with schema setup on the `default` database |
| [`vault`](vault) | Secret storage (`omnia:vault`) |
| [`websocket`](websocket) | Real-time bidirectional messaging (`omnia:websocket`) |

### Composition and deployment patterns

| Example | Demonstrates |
| ------- | ------------ |
| [`http-proxy`](http-proxy) | Outbound HTTP from a guest, including mTLS client certificates |
| [`http-cache`](http-cache) | Outbound HTTP responses cached in an `omnia:keyvalue` bucket through `omnia_sdk::http::cache` |
| [`http-routing`](http-routing) | Two HTTP guests behind path prefixes via a deployment manifest |
| [`guest-link`](guest-link) | Host-mediated guest-to-guest linking by in-memory routing |
| [`cli-static`](cli-static) | A compiled-in command deployment: inline guests, direct-command argv |
| [`mcp`](mcp) | A guest serving MCP tools and resources to AI agents over HTTP |

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

Demos that bind production backends or need external services live in the [`omnia-backends`](https://github.com/augentic/omnia-backends) repo. The [`cursor`](https://github.com/augentic/omnia-backends/tree/main/examples/cursor) demo's guest asks `omnia-cursor` for a live completion and answers the model's function-tool call. The [`otel`](https://github.com/augentic/omnia-backends/tree/main/examples/otel) demo sends guest spans and metrics through a collector to Jaeger and Prometheus. These need extra setup: credentials, CLI tools, containers, or network access.

For a complete application-scale service built on these patterns, see [`omnia-exemplar`](https://github.com/augentic/omnia-exemplar).
