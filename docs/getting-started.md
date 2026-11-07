# Getting Started

This tutorial takes you from a clean checkout to a running WebAssembly guest served over HTTP, then shows the single most important idea in Omnia: the same guest code runs against different backends without recompilation.

Everything here uses in-memory defaults — no databases, brokers, or credentials are needed.

## Prerequisites

- **Rust 1.99 or later.** The repository's `rust-toolchain.toml` pins the toolchain and automatically installs the `wasm32-wasip2` compilation target the first time you build.
- A checkout of this repository.

## Step 1: Build a guest

A **guest** is your application logic, compiled to a WebAssembly component (a `.wasm` file). Build the HTTP example guest:

```bash
cargo build --example http-wasm --target wasm32-wasip2
```

This produces `target/wasm32-wasip2/debug/examples/http_wasm.wasm` (note: the file name uses underscores).

The guest itself is ordinary Rust. In an Omnia service, you write plain [Axum](https://github.com/tokio-rs/axum) handlers on a `Router` whose state is a capability-owning `Client`, and export the WASI HTTP interface with `omnia_sdk::api::http::export!`:

```rust,noplayground
pub fn http_router<P: Send + Sync + 'static>(provider: P) -> Router {
    Router::new()
        .route("/", get(echo_get).post(echo_post))
        .with_state(Client::new(provider))
}

omnia_sdk::api::http::export!(http_router);
```

A handler reaches its capabilities through the `omnia_sdk::api::Context<P>` extractor; `get`, `post`, and the input extractors are axum's own. On `wasm32`, `http::export!` expands to the raw WASI `wasip3::http::service::export!` implementation, calls `http_router(omnia_sdk::WasiProvider)`, and serves each request through `omnia_wasi_http::serve` inside an `http_guest_handle` OpenTelemetry span (the standalone `examples/http/guest.rs` writes that expansion out explicitly so you can see the underlying WASI boundary).

## Step 2: Run it in a host

A **host** is a native binary that loads the guest and provides its capabilities. Run the example host, passing it the `.wasm` file (`-v` shows startup logs):

```bash
cargo run --example http -- -v run ./target/wasm32-wasip2/debug/examples/http_wasm.wasm
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

```rust,noplayground
omnia::runtime!({
    hosts: {
        WasiHttp: HttpDefault,
        WasiOtel: OtelDefault,
    }
});
```

Each entry pairs a **WASI host** (the interface guests see, e.g. `WasiKeyValue`) with a **backend** (the implementation behind it, e.g. `KeyValueDefault`, an in-memory cache). The macro generates `main`, connects the backends, links the interfaces, and starts the servers.

## Step 4: Use a stateful capability

In application handlers, capabilities are traits the context's accessors borrow the provider as (implemented by `WasiProvider` in the guest and by `omnia_test::guest::Provider` in native tests), such as `StateStore` for key-value storage through `context.state()`:

```rust,noplayground
async fn store_item<P: StateStore>(
    context: Context<P>, Json(input): Json<ItemInput>,
) -> Result<Json<ItemReply>, Error> {
    context.state().set("my_key", input.body.as_bytes(), None).await?;
    let res = context.state().get("my_key").await?;
    tracing::debug!("found val: {res:?}");
    // ...
}
```

Under the hood, `WasiProvider` delegates to the standard `wasi:keyvalue` interface, which the standalone `keyvalue` example (`examples/keyvalue/guest.rs`) calls directly to demonstrate the raw WIT binding:

```rust,noplayground
async fn handler(body: Bytes) -> omnia_sdk::Result<Json<Value>> {
    let bucket = store::open("omnia_bucket".to_string()).await.context("opening bucket")?;

    bucket.set("my_key".to_string(), body.to_vec()).await.context("storing data")?;

    let res = bucket.get("my_key".to_string()).await.context("reading data")?;
    tracing::debug!("found val: {res:?}");
```

Build and run it the same way:

```bash
cargo build --example keyvalue-wasm --target wasm32-wasip2
cargo run --example keyvalue -- run ./target/wasm32-wasip2/debug/examples/keyvalue_wasm.wasm
curl -X POST http://localhost:8080 -d '"some data"'
```

The key point: the guest never mentions an implementation. It talks to `wasi:keyvalue`; the host decides whether that means an in-memory cache (here), Redis, or NATS. Swapping the backend is a one-line change in the host — the `.wasm` file does not change. That swap is exactly what [Production Backends](guides/production-backends.md) covers.

## Where to go next

- **[Starting a New Project](new-project.md)** — set up your own guest + host workspace outside this repository.
- **[Writing Guests](guides/writing-guests.md)** — the full guest-side toolkit: more WASI capabilities, tracing, command-mode guests.
- **[Composing a Runtime](guides/composing-a-runtime.md)** — everything the `runtime!` macro accepts and what it generates.
- **[Multi-Guest Deployments](guides/multi-guest-deployments.md)** — run several guests in one runtime with routing and linking.
- Browse [`examples/`](../examples/README.md) — every WASI interface has a runnable example.
