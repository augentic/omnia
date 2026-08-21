# Writing Guests

A guest is your application logic compiled to a WebAssembly component. This guide is an **overview**: it shows each guest-side pattern once and links to the deep dive. Choose your path:

- [Project setup](#project-setup) — the crate shape every guest shares
- [HTTP handlers](#http-handlers) — serve requests with Axum
- [Using WASI capabilities](#using-wasi-capabilities) — storage, messaging, SQL, models, and the rest
- [Handling incoming messages](#handling-incoming-messages) — the messaging trigger
- [The operation contract](#the-operation-contract) — the typed router model behind command mode and typed HTTP/messaging routing
- [Command-mode guests](#command-mode-guests) — run-once jobs and CLIs
- [Tracing](#tracing) — spans and logs from inside the sandbox
- [Serving MCP tools](#serving-mcp-tools) — expose tools to AI agents

Every pattern here is drawn from a runnable pair in [`examples/`](../../examples/).

## Project setup

A guest is a `cdylib` crate targeting `wasm32-wasip2`. Guest code is guarded with `#[cfg(target_arch = "wasm32")]` so the same workspace also compiles for the host triple:

```rust
#![cfg(target_arch = "wasm32")]
```

Typical guest dependencies:

- `wasip3` — WASI Preview 3 bindings (exports, HTTP types, CLI, filesystem preopens)
- `omnia-guest` — guest SDK: `HttpResult`, error types, ORM helpers, MCP support
- `omnia-wasi-*` — the guest side of each capability you use (`omnia-wasi-keyvalue`, `omnia-wasi-messaging`, ...). These crates compile to guest bindings on `wasm32` and to the host implementation on native, so hosts and guests share one dependency name.

A minimal HTTP guest crate looks like this (align `wasip3`/`wit-bindgen` with the versions the omnia workspace pins — a mismatch causes executor deadlocks, see [Troubleshooting](../troubleshooting.md#outbound-http-or-spawned-work-inside-a-handler-deadlocks)):

```toml
[package]
name = "my-guest"
edition = "2024"

[lib]
crate-type = ["cdylib"]

[dependencies]
anyhow = "1"
axum = { version = "0.8", default-features = false, features = ["json"] }
omnia-guest = "0.35"
omnia-wasi-http = "0.35"
serde = { version = "1", features = ["derive"] }
serde_json = "1"
wasip3 = { version = "0.7", features = ["http-compat"] }
wit-bindgen = { version = "0.60", features = ["async-spawn"] }
```

Build with:

```bash
cargo build --example <name>-wasm --target wasm32-wasip2
# output: target/wasm32-wasip2/debug/examples/<name>_wasm.wasm  (underscores)
```

## HTTP handlers

Export the WASI HTTP handler and hand routing to [Axum](https://github.com/tokio-rs/axum) via `omnia_wasi_http::serve`:

```rust,noplayground
struct HttpGuest;
wasip3::http::service::export!(HttpGuest);

impl Guest for HttpGuest {
    #[omnia_wasi_otel::instrument(name = "http_guest_handle", level = Level::DEBUG)]
    async fn handle(request: Request) -> Result<Response, ErrorCode> {
        let router = Router::new().route("/", get(echo_get)).route("/", post(echo_post));
        omnia_wasi_http::serve(router, request).await
    }
}
```

Handlers are ordinary Axum handlers. Return `omnia_guest::HttpResult<T>` to map errors to HTTP responses; `anyhow::Context` works as usual.

For **outbound** HTTP requests, use `omnia_wasi_http::handle` with a standard `http::Request` (see `examples/http-proxy` and the messaging example's upstream call).

## Using WASI capabilities

Each capability is a module in its `omnia-wasi-*` crate. The guest never names an implementation — the host decides what backs each interface.

Key-value (`wasi:keyvalue`):

```rust,noplayground
let bucket = store::open("omnia_bucket".to_string()).await.context("opening bucket")?;

bucket.set("my_key".to_string(), body.to_vec()).await.context("storing data")?;

let res = bucket.get("my_key".to_string()).await.context("reading data")?;
```

Publishing a message (`wasi:messaging`):

```rust
let client = Client::connect("default".to_string()).await?;
let message = Message::new(&payload);
producer::send(&client, "my-topic".to_string(), message).await?;
```

The other capabilities follow the same shape; each has a full example:

| Capability | Guest module | Example | Deep dive |
| ---------- | ------------ | ------- | --------- |
| Key-value | `omnia_wasi_keyvalue::store` | `examples/keyvalue` | — |
| Messaging | `omnia_wasi_messaging::{producer, request_reply}` | `examples/messaging` | [Messaging](messaging.md) |
| SQL + ORM | `omnia_wasi_sql` (with `entity!`) | `examples/sql` | [SQL and the ORM](sql-and-orm.md) |
| Document store | `omnia_wasi_docstore` | `examples/docstore` | [Document Store](document-store.md) |
| Blob store | `omnia_wasi_blobstore` | `examples/blobstore` | — |
| Secrets | `omnia_wasi_vault` | `examples/vault` | — |
| Config | `omnia_wasi_config` | `examples/config` | — |
| Identity/OAuth | `omnia_wasi_identity` | `examples/identity` | — |
| Model completions | `omnia_wasi_model::completion` | `examples/model` | [Model Completions](model-completions.md) |
| WebSockets | `omnia_wasi_websocket` | `examples/websocket` | [Messaging § WebSockets](messaging.md#websockets) |

## Handling incoming messages

A guest can export a messaging handler alongside (or instead of) an HTTP handler. The host's messaging trigger delivers each subscribed message to it:

```rust,noplayground
pub struct Messaging;
omnia_wasi_messaging::export!(Messaging with_types_in omnia_wasi_messaging);

impl omnia_wasi_messaging::incoming_handler::Guest for Messaging {
    async fn handle(message: Message) -> anyhow::Result<(), Error> {
        omnia_guest::api::messaging::handle(&router(), message).await
    }
}
```

`examples/messaging` demonstrates pub-sub, request-reply, and fan-out with the in-memory default backend; the same guest works against Kafka or NATS.

## The operation contract

`omnia-guest` keeps application logic independent of how it is invoked. Three pieces, defined once and reused by every transport:

- An **operation** is one unit of application work with a typed input and output (e.g. `CreateItem`).
- A **provider** is the struct your operations run against — it carries their capabilities (implement `DocumentStore`, `Config`, etc. on it).
- An **invoker** (`Invoker::new(owner, provider)`) binds the provider to an owner id and executes operations.

Write an operation as a bare `async fn` and let `#[omnia_guest::operation]` derive the `Operation` impl from its signature — the first parameter is the input type (and the impl target), the second must be `CallContext<'_, P>`, and the return type is `Result<T>` (`omnia_guest::Result`) or `Result<T, E>`:

```rust,noplayground
#[omnia_guest::operation]
async fn create_item<P>(input: CreateItem, context: CallContext<'_, P>) -> Result<ItemReply>
where
    P: Provider + Config + StateStore,
{
    // context.provider carries the capabilities in the fn's bounds
}
```

The macro takes no arguments and adds no instrumentation. The function is re-emitted unchanged, attributes included, so other handlers can still call it directly and a span is added by stacking `#[tracing::instrument]` on the fn (`fields(...)` may reference the `input` and `context` parameters). A hand-written `impl Operation<P>` remains the escape hatch for shapes the macro doesn't cover (e.g. `Input != Self`).

For a guest that runs on the WASI-backed capability defaults, declare the provider with `omnia_guest::provider!` instead of writing one empty impl per capability (the expansion compiles on `wasm32` only; native tests supply mock providers):

```rust,noplayground
omnia_guest::provider! {
    /// Bare provider backed by the default WASI capability implementations.
    pub struct Provider: Config + HttpRequest + Identity + Publish + StateStore;
}
```

Routers then map transport events onto operations: an HTTP router maps method + path, a messaging router maps exact topics, a command router maps CLI subcommands. Your WASI export stays visible application code — it just hands the event to the router:

```rust
fn router() -> omnia_guest::api::http::Router<MyProvider> {
    Router::new(Invoker::new("acme-corp", MyProvider))
        .route("/api/items", post::<CreateItem, MyProvider>())
}

impl Guest for Http {
    async fn handle(request: Request) -> Result<Response, ErrorCode> {
        omnia_guest::api::http::serve(router(), request).await
    }
}
```

Messaging uses `api::messaging::Router` and `consume::<Operation>()`; topic matching is exact, and each route can replace its payload decoder and output/error projector. The export remains visible application code and calls `api::messaging::handle`. Because the same operation types register in any router, one guest can expose the same logic over HTTP, messaging, and a CLI without duplicating it.

## Command-mode guests

For run-once workloads (jobs, CLIs, agent tasks), use `omnia_guest::api::command` to bind Clap argument types to the same [operation contract](#the-operation-contract). The guest still owns the explicit `wasi:cli/run` export; the adapter initializes and flushes guest telemetry, writes buffered output, and preserves the router's exact exit status:

```rust,noplayground
use clap::Command;
use omnia_guest::api::command::{self, Router, RouterBuilder};
use omnia_guest::api::invoke::Invoker;
use wasip3::exports::cli::run::Guest;

fn router() -> Router<MyProvider> {
    RouterBuilder::new(Command::new("jobs"), Invoker::new("acme", MyProvider))
        .route(
            ["sync"],
            command::run::<SyncArgs, Sync>()
                .about("Synchronize records")
                .project_with(Text),
        )
        .build()
        .expect("command routes are valid")
}

struct Cli;
wasip3::cli::command::export!(Cli);

impl Guest for Cli {
    async fn run() -> Result<(), ()> {
        command::execute_wasi(&router()).await
    }
}
```

- Arguments after `--` on the host command line arrive as the guest's argv (`args[0]` is the program name, supplied by the runtime).
- Each route explicitly decodes arguments into an operation input and projects output, operation failures, and decode failures into `CommandResponse`.
- The router supplies nested help, version and usage handling, shell completions, and a read-only route inventory.
- The host runtime must be built with `mode: command` — see [Composing a Runtime](composing-a-runtime.md).

## Tracing

Annotate functions with `#[omnia_wasi_otel::instrument]` to wrap them in an OpenTelemetry span. `tracing::debug!` and friends work inside guests; spans flow to whatever OTel backend the host configures:

```rust
#[omnia_wasi_otel::instrument(name = "http_guest_handle", level = Level::INFO)]
async fn handle(request: Request) -> Result<Response, ErrorCode> { /* ... */ }
```

## Serving MCP tools

A guest can act as an [MCP](https://modelcontextprotocol.io) (Model Context Protocol) server — exposing tools and resources to AI agents over HTTP. Implement `omnia_guest::mcp::McpServer` and serve `mcp::router` from your HTTP handler; see [Model Completions and MCP](model-completions.md#serving-mcp-tools-from-a-guest).
