# Getting Started

This tutorial takes you from a clean checkout to a running WebAssembly guest served over HTTP, then shows the single most important idea in Omnia: the same guest code runs against different backends without recompilation.

Everything here uses in-memory defaults — no databases, brokers, or credentials are needed.

## Prerequisites

- **Rust 1.95 or later.** The repository's `rust-toolchain.toml` pins the toolchain and automatically installs the `wasm32-wasip2` compilation target the first time you build.
- A checkout of this repository.

## Step 1: Build a guest

A **guest** is your application logic, compiled to a WebAssembly component (a `.wasm` file). Build the HTTP example guest:

```bash
cargo build --example http-wasm --target wasm32-wasip2
```

This produces `target/wasm32-wasip2/debug/examples/http_wasm.wasm` (note: the file name uses underscores).

The guest itself is ordinary Rust. It exports a WASI HTTP handler and uses [Axum](https://github.com/tokio-rs/axum) for routing, exactly as you would in a native web service:

```rust
{{#include ../examples/http/guest.rs:19:27}}
```

## Step 2: Run it in a host

A **host** is a native binary that loads the guest and provides its capabilities. Run the example host, passing it the `.wasm` file:

```bash
export RUST_LOG=info
cargo run --example http -- run ./target/wasm32-wasip2/debug/examples/http_wasm.wasm
```

When the log line `omnia ready` appears, the runtime is serving on `localhost:8080`. Try it from another terminal:

```bash
curl -X POST http://localhost:8080 \
  -H "Content-Type: application/json" \
  -d '{"hello": "world"}'
```

You should get the guest's echo response back as JSON.

What just happened: the host linked a WASI HTTP implementation into a wasmtime engine, pre-instantiated your guest, and started an HTTP server. Each incoming request instantiates a fresh, isolated guest instance — nothing leaks between requests.

## Step 3: Look at the host

The entire host is one macro invocation. This is the runtime you just ran:

```rust
{{#include ../examples/http/runtime.rs:8:13}}
```

Each entry pairs a **WASI host** (the interface guests see, e.g. `WasiKeyValue`) with a **backend** (the implementation behind it, e.g. `KeyValueDefault`, an in-memory cache). The macro generates `main`, connects the backends, links the interfaces, and starts the servers.

## Step 4: Use a stateful capability

The `keyvalue` example adds storage. Its guest opens a bucket and reads/writes keys through the standard `wasi:keyvalue` interface:

```rust
{{#include ../examples/keyvalue/guest.rs:41:47}}
```

Build and run it the same way:

```bash
cargo build --example keyvalue-wasm --target wasm32-wasip2
cargo run --example keyvalue -- run ./target/wasm32-wasip2/debug/examples/keyvalue_wasm.wasm
curl -X POST http://localhost:8080 -d '"some data"'
```

The key point: the guest never mentions an implementation. It talks to `wasi:keyvalue`; the host decides whether that means an in-memory cache (here), Redis, or NATS. Swapping the backend is a one-line change in the host — the `.wasm` file does not change. That swap is exactly what [Production Backends](guides/production-backends.md) covers.

## Where to go next

- **[Writing Guests](guides/writing-guests.md)** — the full guest-side toolkit: more WASI capabilities, tracing, command-mode guests.
- **[Composing a Runtime](guides/composing-a-runtime.md)** — everything the `runtime!` macro accepts and what it generates.
- **[Multi-Guest Deployments](guides/multi-guest-deployments.md)** — run several guests in one runtime with routing and linking.
- Browse [`examples/`](../examples/README.md) — every WASI interface has a runnable example.
