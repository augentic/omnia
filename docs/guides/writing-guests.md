# Writing Guests

A guest is your application logic compiled to a WebAssembly component. This guide is an **overview**: it shows each guest-side pattern once and links to the deep dive. Choose your path:

- [Project setup](#project-setup) — the crate shape every guest shares
- [The handler contract](#the-handler-contract) — Handler + Client + Context behind typed HTTP/messaging routing and CLI dispatch
- [HTTP handlers](#http-handlers) — serve requests with Axum and `http::export!`
- [Using WASI capabilities](#using-wasi-capabilities) — storage, messaging, SQL, models, and the rest via `WasiProvider`
- [Handling incoming messages](#handling-incoming-messages) — the messaging trigger and `messaging::export!`
- [Command-mode guests](#command-mode-guests) — run-once jobs and CLIs
- [Tracing](#tracing) — spans and logs from inside the sandbox
- [Serving MCP tools](#serving-mcp-tools) — expose tools to AI agents

Every pattern here is drawn from [`omnia-exemplar`](https://github.com/augentic/omnia-exemplar) and the runnable pairs in [`examples/`](../../examples/).

## Project setup

A guest is a library crate with `crate-type = ["cdylib", "rlib"]`: `cdylib` produces the WebAssembly component on `wasm32-wasip2`, and `rlib` lets native integration tests link your routers and handlers against `omnia_test::guest::Provider` doubles.

Typical guest dependencies:

- `omnia-sdk` — the guest SDK: the handler contract, `WasiProvider`, HTTP/messaging routers and the command façade, each transport's `export!` macro (`http::export!`, `messaging::export!`, `command::export!`), error types, capability traits, and MCP support
- `axum`, `serde`, `serde_json` — routing and typed request/response serialization

A minimal HTTP guest `Cargo.toml` looks like this:

```toml
[package]
name = "my-guest"
version = "0.1.0"
edition = "2024"

[lib]
crate-type = ["cdylib", "rlib"]

[dependencies]
axum = { version = "0.8", default-features = false, features = ["json"] }
omnia-sdk = "0.38"
serde = { version = "1", features = ["derive"] }
serde_json = "1"
```

`omnia-sdk` re-exports `wasip3`, `wit-bindgen`, and the `omnia-wasi-*` guest bindings internally for the `export!` macro's expansion on `wasm32`, so you do not need to list `wasip3` or `wit-bindgen` in `Cargo.toml` or gate your crate with `#![cfg(target_arch = "wasm32")]`. (If you write a raw WASI export or call `omnia-wasi-*` modules directly as the standalone `examples/` do, gate wasm-only modules with `#![cfg(target_arch = "wasm32")]` and align `wasip3`/`wit-bindgen` with the versions the omnia workspace pins — see [Troubleshooting](../troubleshooting.md#outbound-http-or-spawned-work-inside-a-handler-deadlocks).)

Build a guest with:

```bash
cargo build -p my-guest --target wasm32-wasip2
# or for an in-repo example:
cargo build --example <name>-wasm --target wasm32-wasip2
# output: target/wasm32-wasip2/debug/examples/<name>_wasm.wasm  (underscores)
```

## The handler contract

`omnia-sdk` keeps application logic independent of how it is invoked. Three pieces, defined once:

- A **provider** is the struct your handlers run against — it carries their capabilities (implement `DocumentStore`, `Config`, etc. on it).
- A **client** (`Client::new(provider)`) owns the provider. It is the state of an HTTP router and the thing a messaging router or command invokes handlers through.
- A **context** (`Context<P>`) is what a handler receives: `provider()` and the public transport-neutral `metadata` (request, correlation, and causation ids, and the tenant when the transport names one). A handler bounds its `P` on the capability traits it uses and never names the provider.

An **HTTP handler** is a plain [Axum](https://github.com/tokio-rs/axum) handler. `Context<P>` is an extractor over the `Client` in the router state; the input comes through axum's own `Path`, `Query`, and `Json` extractors; the output is anything `IntoResponse`, typically `Json<Reply>`:

```rust,noplayground
async fn create_item<P>(
    context: Context<P>, Json(input): Json<CreateItem>,
) -> Result<Json<ItemReply>, Error>
where
    P: Config + StateStore + 'static,
{
    // context.provider() carries the capabilities in the fn's bounds
}
```

A **messaging or command handler** is an `async fn(I, Context<P>) -> Result<O, E>` named for the operation, taking the input DTO by value. Every fn of that shape is a `Handler<P, I>` through a blanket impl, as is any `Clone` closure of it — so a closure can carry configuration into a route. Implementing `Handler<P, I>` by hand on a local non-fn type is the escape hatch for the rare case a fn cannot express. To unit-test a fn that takes a context without a `Client` or a router, build its context with `Context::new(provider, metadata)`.

Logic shared by an HTTP route and a topic lives in a plain fn both call: the axum handler extracts and hands over, the messaging handler decodes and hands over.

For a guest that runs on the WASI-backed capability defaults, `omnia_sdk::WasiProvider` is a zero-sized struct that implements every capability trait on `wasm32`. Native handler tests run the same handlers against `omnia_test::guest::Provider` (see [Provider Portability](portability.md) and [Testing Omnia-Based Code](testing-omnia-code.md)):

```rust,noplayground
pub fn http_router<P>(provider: P) -> axum::Router
where
    P: Config + HttpRequest + Identity + Publish + StateStore + 'static,
{
    axum::Router::new()
        .route("/api/items", post(create_item))
        .with_state(Client::new(provider))
}

omnia_sdk::api::http::export!(http_router);
```

Routers map transport events onto handlers: an HTTP route is registered through axum's own `get`/`post`/`put`/`patch`/`delete` on an `axum::Router` whose state is one provider-owning `Client`, and a messaging router maps exact topics. Each transport's `export!` macro (`http::export!`, `messaging::export!`, or `command::export!`) binds its WASI export, calling the router fn with `WasiProvider` (or a custom provider passed as a second argument), and expands to the `#[omnia_wasi_otel::instrument]`-scoped export on `wasm32` and to nothing on native targets. The HTTP export also layers `api::http::trace`, which serves each request inside a `handler` span carrying the request and correlation ids; a native test can layer it with `axum::middleware::from_fn(trace)`.

Messaging uses `api::messaging::Router` and `route("items.created", item_created)`; topic matching is exact. A module that owns topics registers them on an `api::messaging::Routes` of its own, with no client, and the guest merges each (`Routes::from([..])` or `Router::merge`) and closes the whole into its one `Router` with `Routes::with_state`, as a module's routes are an `api::http::Routes<P>` merged (`http::Routes::from([..])`) and closed into an `api::http::Router` by `with_state`; a topic registered twice panics where the two meet. `Routes::map` rewrites every topic in one pass, so a module registers bare topic names and the root qualifies them once with the deployment's prefix. `route` decodes JSON; `route_with(topic, handler, decode)` takes a decoder `Fn(&Delivery) -> Result<I, DecodeError>` over the whole `Delivery` for any other wire format.

### Errors

An HTTP handler's error is anything `IntoResponse`. An `omnia_sdk::Error` (`bad_request!`, `not_found!`, `server_error!`, `bad_gateway!`) becomes the JSON `api::ErrorBody` (`{"error","message"}`) at the variant's status, the same envelope the command transport writes to stderr; `?` on an `anyhow::Error` or a `serde_json::Error` converts into it. A domain error with its own wire shape (another status, another content type such as an XML fault) implements `IntoResponse` itself and delegates its host-failure variant to `omnia_sdk::Error::into_response`. A malformed body or query is refused by axum's extractor before the handler runs (`400`, or `415` without `Content-Type: application/json`).

## HTTP handlers

Register handlers on an [Axum](https://github.com/tokio-rs/axum) `Router` under axum's `get`/`post`/`put`/`patch`/`delete`, and export the router with `http::export!`:

```rust,noplayground
use axum::Router;
use axum::routing::get;
use omnia_sdk::api::{Client, http};

pub fn http_router<P: Send + Sync + 'static>(provider: P) -> Router {
    Router::new()
        .route("/", get(echo_get).post(echo_post))
        .with_state(Client::new(provider))
}

http::export!(http_router);
```

For **outbound** HTTP requests inside a handler, bound `P: HttpRequest` and call `context.provider().fetch(request).await` with a standard `http::Request` (or call `omnia_wasi_http::handle` directly in `wasm32`-only code; see `examples/http-proxy`).

> **Advanced: raw WASI HTTP export.** When you need to customize the `wasi:http/incoming-handler` export directly (as the standalone `examples/` do to show the low-level binding), implement `wasip3::exports::http::handler::Guest` and hand the request to `omnia_wasi_http::serve`:
>
> ```rust,noplayground
> struct HttpGuest;
> wasip3::http::service::export!(HttpGuest);
>
> impl Guest for HttpGuest {
>     #[omnia_wasi_otel::instrument(name = "http_guest_handle", level = Level::DEBUG)]
>     async fn handle(request: Request) -> Result<Response, ErrorCode> {
>         let router = Router::new().route("/", get(echo_get)).route("/", post(echo_post));
>         omnia_wasi_http::serve(router, request).await
>     }
> }
> ```

## Using WASI capabilities

In handlers, access host capabilities through the portable traits on `context.provider()` — `WasiProvider` implements every trait on `wasm32` over the underlying `omnia-wasi-*` WIT bindings, while `omnia_test::guest::Provider` backs them with in-memory doubles in native tests. The guest never names a backend implementation — the host decides what backs each interface.

Key-value (`StateStore` over `wasi:keyvalue`):

```rust,noplayground
context.provider().set("my_key", &body, None).await?;
let res = context.provider().get("my_key").await?;
```

Publishing a message (`Publish` over `wasi:messaging`):

```rust,noplayground
let message = Message::new(&payload);
context.provider().send("my-topic", &message).await?;
```

When writing `wasm32`-only code (as in `examples/`), you can also call the `omnia-wasi-*` guest modules directly:

| Capability | `WasiProvider` trait (`omnia_sdk`) | Raw guest module | Example | Deep dive |
| ---------- | ---------------------------------- | ---------------- | ------- | --------- |
| Key-value | `StateStore` | `omnia_wasi_keyvalue::store` | `examples/keyvalue` | — |
| Messaging | `Publish` | `omnia_wasi_messaging::{producer, request_reply}` | `examples/messaging` | [Messaging](messaging.md) |
| SQL | `TableStore` (feature `sql`) | `omnia_wasi_sql` | `examples/sql` | [SQL](sql.md) |
| Document store | `DocumentStore` (feature `docstore`) | `omnia_wasi_docstore` | `examples/docstore` | [Document Store](document-store.md) |
| Blob store | `BlobStore` | `omnia_wasi_blobstore` | `examples/blobstore` | — |
| Secrets | `Vault` | `omnia_wasi_vault` | `examples/vault` | — |
| Config | `Config` | `omnia_wasi_config` | `examples/config` | — |
| Identity/OAuth | `Identity` | `omnia_wasi_identity` | `examples/identity` | — |
| Outbound HTTP | `HttpRequest` | `omnia_wasi_http` | `examples/http-proxy` | — |
| Model completions | `Model` | `omnia_wasi_model::completion` | `examples/model` | [Model Completions](model-completions.md) |
| Version control | `Vcs` | `omnia_wasi_vcs` | — | — |
| Dynamic plugins | `Plugins` | `omnia_sdk::plugins` | — | — |
| WebSockets | `Broadcast` | `omnia_wasi_websocket` | `examples/websocket` | [Messaging § WebSockets](messaging.md#websockets) |

## Handling incoming messages

A guest can export a messaging handler alongside (or instead of) an HTTP handler. Build an `omnia_sdk::api::messaging::Router` over your provider and export it with `messaging::export!`:

```rust,noplayground
use omnia_sdk::api::Client;
use omnia_sdk::api::messaging::{self, Router};

pub async fn messaging_router<P>(provider: P) -> Router<P>
where
    P: StateStore + Clone + Send + Sync + 'static,
{
    Router::new(Client::new(provider)).route("items.created", on_item_created)
}

messaging::export!(messaging_router);
```

For guests that need direct control over the raw `wasi:messaging/incoming-handler` export (for example, `examples/messaging` demonstrating request-reply and fan-out), implement `omnia_wasi_messaging::incoming_handler::Guest` and dispatch via `omnia_sdk::api::messaging::handle`:

```rust,noplayground
pub struct Messaging;
omnia_wasi_messaging::export!(Messaging with_types_in omnia_wasi_messaging);

impl omnia_wasi_messaging::incoming_handler::Guest for Messaging {
    async fn handle(message: Message) -> anyhow::Result<(), Error> {
        omnia_sdk::api::messaging::handle(&router(), message).await
    }
}
```

`examples/messaging` demonstrates pub-sub, request-reply, and fan-out with the in-memory default backend; the same guest works against Kafka or NATS.

## Command-mode guests

For run-once workloads (jobs, CLIs, agent tasks), `omnia_sdk::api::command` is the command-line transport over the [handler contract](#the-handler-contract): argv is decoded into handler input, the handler runs through `Client::call`, and the output is encoded onto the process channels. The clap-backed parts (`parse`, `completions`, the `clap::ValueEnum` derive on `Format`) sit behind the `command` cargo feature (`omnia-sdk = { version = "...", features = ["command"] }`); the projector itself needs no feature.

```rust,noplayground
use std::fmt;

use clap::{Args, Parser, Subcommand};
use omnia_sdk::Error;
use omnia_sdk::api::command::{self, Command, Parsed, Response, parse};
use omnia_sdk::api::{Client, Context, Format, Metadata};
use serde::Serialize;

#[derive(Parser)]
struct App {
    /// Output format for every verb
    #[arg(long, global = true, default_value = "text")]
    format: Format,

    #[command(subcommand)]
    verb: Verb,
}

#[derive(Subcommand)]
enum Verb {
    Sync(SyncInput),
}

#[derive(Args)]
struct SyncInput {
    source: String,
}

#[derive(Serialize)]
struct Synced {
    count: usize,
}

async fn sync(input: SyncInput, context: Context<MyProvider>) -> Result<Synced, Error> {
    // ...
}

fn render_synced(synced: &Synced, out: &mut dyn fmt::Write) -> fmt::Result {
    writeln!(out, "synced {} items", synced.count)
}

command::export!(main);

async fn main() -> Response {
    let app = match parse::<App>(wasip3::cli::environment::get_arguments()) {
        Parsed::App(app) => app,
        Parsed::Display(text) => return Response::success(text),
        Parsed::Usage(error) => return Response::usage(&error),
    };
    let client = Client::new(MyProvider);
    let metadata = Metadata::from_env("APP");
    let command = Command::new(&client, &metadata, app.format)
        .hints(|error| (error.code() == "not_found").then(|| "run `app sync --help`".into()));
    match app.verb {
        Verb::Sync(input) => command.call(sync, || Ok(input), render_synced).await,
    }
}
```

The pieces, in the order argv flows through them:

- **`parse::<App>(argv) -> Parsed`** classifies clap's outcomes: the grammar (`Parsed::App`), help or version text for stdout at exit 0 (`Parsed::Display`), or a usage error (`Parsed::Usage`), which `Response::usage(&error)` renders on stderr at `USAGE_EXIT` (64, `EX_USAGE`). `completions::<App>(shell, name)` produces a shell-completion script as a `Response`; `Shell` is re-exported from `clap_complete`.
- **`Command::new(&client, &metadata, format)`** is the projector, and `command.call(handler, decode, render)` runs one verb. `decode: FnOnce() -> Result<I, Error>` builds the handler input from the parsed grammar (its `bad_request!` exits 1 like any other refusal); the handler runs through `Client::call`; a success body is encoded in the selected `Format` onto stdout at exit 0 — `render` is its text form (`Fn(&T, &mut dyn fmt::Write) -> fmt::Result`), and `Format::Json` is the output pretty-printed with a trailing newline. `.hints(|error| ..)` attaches a remedy hint to every failure that carries none.
- **`Response { stdout, stderr, exit }`** buffers both channels. `command::export!(main)` binds an `async fn main() -> Response` as the `wasi:cli/run` export and writes the channels at that boundary through `IntoExit`: a `BrokenPipe` on either channel (a reader that went away) keeps the response's own exit, any other refused channel exits 3. Because the channels are whole buffers, a command that must stream writes to the process channels itself and returns `Result<(), u8>` or `()`, which the export also accepts; a guest that needs its own export writes the raw `wasip3` export and calls `command::execute_wasi` itself.
- **`Metadata::from_env(prefix)`** reads `<PREFIX>_REQUEST_ID`, `<PREFIX>_CORRELATION_ID`, `<PREFIX>_CAUSATION_ID`, and `<PREFIX>_TENANT` from the environment. A missing request id is minted from `wasi:random`, as on every transport, so a command invocation is as observable as an HTTP request: `Client::call` runs the handler in a `handler` span carrying the request and correlation ids.

### Failures and the exit map

A decode or handler error becomes a `Failure` envelope on stderr, in the same `Format` as a success body, and the process exits with `Error::exit_code()`:

| Error class | `error` (from the macros) | Exit | HTTP status |
| ----------- | ------------------------- | ---- | ----------- |
| `BadRequest` | `bad_request` | 1 | 400 |
| `NotFound` | `not_found` | 2 | 404 |
| `ServerError` | `server_error` | 3 | 500 |
| `BadGateway` | `bad_gateway` | 4 | 502 |
| clap usage error | — | 64 | — |

As text the envelope is `error[<code>]: <message>` followed by `hint: <hint>` when a hint is attached; as JSON it is flat: `{"error","message","exit-code","hint"?}` (the hint key is omitted when absent). `error` and `message` are the transport-neutral `api::ErrorBody` — `Error::into_response` emits the same two fields as an `application/json` body at the variant's status — so a client reads one discriminant whether it reached the handler over HTTP or a shell. Usage errors exit 64 rather than clap's default 2, so exit 2 always means a `NotFound` envelope.

- Arguments after `--` on the host command line arrive as the guest's argv (`args[0]` is the program name, supplied by the runtime).
- The host runtime must be built with `mode: command` — see [Composing a Runtime](composing-a-runtime.md).
- The telemetry lifecycle imports `omnia:otel`, so the deployment links `WasiOtel` (the no-op `OtelDefault` suffices).

`examples/cli` is the runnable pair: `greet`, `add`, `env`, and a `fail [CLASS]` verb that returns each error class so the exit map can be observed.

## Tracing

Annotate functions with `#[omnia_wasi_otel::instrument]` to wrap them in an OpenTelemetry span. `tracing::debug!` and friends work inside guests; spans flow to whatever OTel backend the host configures:

```rust
#[omnia_wasi_otel::instrument(name = "http_guest_handle", level = Level::INFO)]
async fn handle(request: Request) -> Result<Response, ErrorCode> { /* ... */ }
```

The outermost instrumented `async fn` (or `command::export!`, which does the same for a CLI entry) runs inside `omnia_wasi_otel::scope`: it initializes the guest's telemetry on entry and exports every span and metric as it returns, so put the attribute on the export's `handle` and on any handler beneath it. A sync function only opens a span.

The OpenTelemetry API works directly too: `global::tracer(..)` and `global::meter(..)` return Omnia's own providers, which record straight into the `omnia:otel` records the flush exports — no OpenTelemetry SDK is linked into the guest. Counters, histograms, and gauges export delta series (one instance per invocation); up-down counters are cumulative. Observable (callback) instruments are no-ops: record through a synchronous instrument instead.

Console output (events, never a span prefix) goes to stderr, so stdout stays the guest's own; it follows the guest's `RUST_LOG`, defaulting to `error`. Spans and metrics are exported whatever the filter admits: a span below the filter's level is never created, so an `#[instrument]` at `INFO` needs `RUST_LOG=info` to reach the host. A guest's `RUST_LOG` is the one its WASI environment carries: the runtime's tracing filter — a `-v`/`-q` flag's level, else the process `RUST_LOG`'s, else `info` for a command and `warn` for a server, with the process `RUST_LOG`'s targeted directives (`my_guest=debug`) on top (see [Verbosity flags](../reference/configuration.md#verbosity-flags)) — so every guest a runtime dispatches opens at the same level without naming another's, and an operator's `RUST_LOG=my_guest=trace` refines one guest under any flag. A command guest declares the flags by flattening `omnia_sdk::api::command::Verbosity` into its grammar; it never acts on them.

## Serving MCP tools

A guest can act as an [MCP](https://modelcontextprotocol.io) (Model Context Protocol) server — exposing tools and resources to AI agents over HTTP. Implement `omnia_sdk::mcp::McpServer` and serve `mcp::router` from your HTTP handler; see [Model Completions and MCP](model-completions.md#serving-mcp-tools-from-a-guest).
