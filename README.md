# OMNIA — Quick WebAssembly Secure Runtime

OMNIA provides a thin wrapper around [`wasmtime`](https://github.com/bytecodealliance/wasmtime) for ergonomic integration of host-based services for WASI components.

While it can be used standalone, Omnia is primarily intended for use with Augentic's Agent Skills. It provides a safe, hand-crafted runtime for safely running agent-generated WASI components.

The  opinionated nature of WASI guest components and more particularly, the Omnia framework, provides a level of control and consistency hard to achieve in agent-generated code.

## Examples

There are a number of examples provided in the `examples` directory that can be used to experiment with the runtime and see it in action.

Each example contains a Wasm guest and the runtime required to run it.

See [examples/README.md](./examples/README.md) for more details.

## Building

There are multiple ways to build a runtime by combining `--bin` and `--features` flags.
For example, to build the `omnia` runtime with all features enabled:

```bash
cargo build --bin=omnia --features=omnia --release
```

### Docker

Building with Docker:

```bash
export CARGO_REGISTRIES_AUGENTIC_TOKEN="<registry token>"

docker build \
  --build-arg BIN="omnia" \
  --build-arg FEATURES="omnia" \
  --secret id=augentic,env=CARGO_REGISTRIES_AUGENTIC_TOKEN \
  --tag ghcr.io/augentic/omnia .
```

## Crates

| Crate | Description |
|-------|-------------|
| [`omnia`](crates/omnia) | Core runtime -- wasmtime wrapper with CLI and pluggable WASI host services |
| [`omnia-sdk`](crates/omnia-sdk) | Guest SDK -- traits, error types, and macros for WASI component authors |
| [`omnia-orm`](crates/orm) | ORM layer for wasi-sql with fluent query builder |
| [`omnia-otel`](crates/otel) | OpenTelemetry tracing and metrics for the runtime |
| [`omnia-guest-macro`](crates/guest-macro) | `guest!` proc-macro for guest HTTP/messaging handlers |
| [`omnia-runtime-macro`](crates/runtime-macro) | `runtime!` proc-macro for host runtime generation |
| [`omnia-wasi-blobstore`](crates/wasi-blobstore) | wasi:blobstore host and guest bindings |
| [`omnia-wasi-config`](crates/wasi-config) | wasi:config host and guest bindings |
| [`omnia-wasi-http`](crates/wasi-http) | wasi:http host and guest bindings |
| [`omnia-wasi-identity`](crates/wasi-identity) | wasi:identity host and guest bindings |
| [`omnia-wasi-keyvalue`](crates/wasi-keyvalue) | wasi:keyvalue host and guest bindings |
| [`omnia-wasi-messaging`](crates/wasi-messaging) | wasi:messaging host and guest bindings |
| [`omnia-wasi-otel`](crates/wasi-otel) | wasi:otel host and guest bindings |
| [`omnia-wasi-otel-attr`](crates/wasi-otel-attr) | `#[instrument]` attribute macro for WASI otel |
| [`omnia-wasi-sql`](crates/wasi-sql) | wasi:sql host and guest bindings |
| [`omnia-wasi-vault`](crates/wasi-vault) | wasi:vault host and guest bindings |
| [`omnia-wasi-websocket`](crates/wasi-websocket) | wasi:websocket host and guest bindings |
