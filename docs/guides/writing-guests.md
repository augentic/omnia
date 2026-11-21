# Writing Guests

A guest is your application logic compiled to a WebAssembly component. This guide is an **overview**: it shows each guest-side pattern once and links to the deep dive. Choose your path:

- [Project setup](#project-setup) — the crate shape every guest shares
- [The handler contract](#the-handler-contract) — `Handlers`, `Router`, and `Context` behind typed HTTP/messaging routing and CLI dispatch
- [HTTP handlers](#http-handlers) — serve requests with Axum and `http::export!`
- [Using host interfaces](#using-host-interfaces) — storage, messaging, SQL, models, and the rest through the generated bindings
- [Handling incoming messages](#handling-incoming-messages) — the messaging trigger and `messaging::export!`
- [Command-mode guests](#command-mode-guests) — run-once jobs and CLIs
- [Tracing](#tracing) — spans and logs from inside the sandbox
- [Serving MCP tools](#serving-mcp-tools) — expose tools to AI agents

Every pattern here is drawn from [`omnia-exemplar`](https://github.com/augentic/omnia-exemplar) and the runnable pairs in [`examples/`](../../examples/).

## Project setup

A guest is a library crate with `crate-type = ["cdylib", "rlib"]`: `cdylib` produces the WebAssembly component on `wasm32-wasip3`, and `rlib` lets another crate link its handlers. The crate also compiles natively, so `cargo check` and `cargo clippy` run without a `--target`. A host call made natively panics, so a test runs the compiled component (see [Testing Omnia-Based Code](testing-omnia-code.md)).

Typical guest dependencies:

- `omnia-sdk` — the guest SDK: the generated bindings for every `omnia:*` interface and the helpers over them, the one `Handlers`/`Router` pair over HTTP routes, messaging topics, and CLI subcommands, each transport's `export!` macro (`http::export!`, `messaging::export!`, `command::export!`), error types, and MCP support
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
omnia-sdk = "0.39"
serde = { version = "1", features = ["derive"] }
serde_json = "1"
```

`omnia-sdk` re-exports `wasip3` for the `export!` macros' expansion on `wasm32`, so you do not need to list `wasip3` or `wit-bindgen` in `Cargo.toml` or gate your crate with `#![cfg(target_arch = "wasm32")]`. A guest that writes a raw WASI export, as the standalone `examples/` do, depends on `wasip3` itself and gates its wasm-only modules with `#![cfg(target_arch = "wasm32")]`. Its `wasip3` must be the version `omnia-sdk` pins (see [Troubleshooting](../troubleshooting.md#outbound-http-or-spawned-work-inside-a-handler-deadlocks)).

A raw export that may wait must be an `async func` in its WIT. On `wasm32-wasip3`, `std::thread::sleep` and every async-typed import suspend the task, and a sync-typed (`func`) export that suspends traps — see [Troubleshooting](../troubleshooting.md#cannot-block-a-synchronous-task-before-returning). The exports behind the `export!` macros are already async-typed.

Build a guest with:

```bash
cargo build -p my-guest --target wasm32-wasip3
# or for an in-repo example:
cargo build --example <name>-wasm --target wasm32-wasip3
# output: target/wasm32-wasip3/debug/examples/<name>_wasm.wasm  (underscores)
```

## The handler contract

`omnia-sdk` keeps application logic independent of how it is invoked. A handler is an `async fn` over its input, and over a **context** when it reads the invocation's metadata:

- A **context** (`Context`) carries the public transport-neutral `metadata`: the request, correlation, and causation ids, the tenant when the transport names one, and the deadline the host set for the invocation.
- A handler reaches the host through the interface modules (`keyvalue`, `sql`, `messaging`, ...), never through the context. Each opens a resource on a binding the deployment declares (see [Using host interfaces](#using-host-interfaces)).

An **HTTP handler** is a plain [Axum](https://github.com/tokio-rs/axum) handler. `Context` is an extractor over any router state; the input comes through axum's own `Path`, `Query`, and `Json` extractors; the output is anything `IntoResponse`, typically `Json<Reply>`:

```rust,noplayground
async fn create_item(Json(input): Json<CreateItem>) -> Result<Json<ItemReply>, Error> {
    let bucket = keyvalue::default().await?;
    // ...
}
```

A **messaging or command handler** is an `async fn(I) -> Result<O, E>` named for the operation, taking the input DTO by value. One that reads the metadata is an `async fn(Context, I) -> Result<O, E>`, taking the context first as an HTTP handler does. Every fn of either shape is a `Handler<I, M>` through a blanket impl, where the inferred `M` is the fn's argument list, as is any `Clone` closure of it — so a closure can carry configuration into a route. Implementing `Handler<I, M>` by hand on a local non-fn type is the escape hatch for the rare case a fn cannot express. To call a fn that takes a context without a router, build its context with `Context::new(metadata)`.

A module registers its handlers on a `Handlers` of its own, and the guest root merges each module's and closes the whole into its one `Router`:

```rust,noplayground
// crates/orders/src/lib.rs
pub fn handlers() -> Handlers {
    Handlers::from([placing::handlers(), catalogue::handlers()])
}

// src/lib.rs, the guest root
fn router() -> Router {
    Handlers::from([orders::handlers(), places::handlers()]).into_router()
}

omnia_sdk::http::export!(router);
```

Logic shared by an HTTP route and a topic lives in a plain fn both call: the axum handler extracts and hands over, the messaging handler decodes and hands over.

One `Handlers` maps every transport's events onto handlers: an HTTP route is registered through axum's own `get`/`post`/`put`/`patch`/`delete` (`route`), a messaging topic by exact name (`topic`), and a CLI subcommand by name (`command`), and `into_router()` closes them into the one `Router`. Each transport's `export!` macro (`http::export!`, `messaging::export!`, or `command::export!`) binds its WASI export, calling the router fn on each invocation. It expands to the telemetry-scoped export on `wasm32` and binds nothing on native targets. The HTTP export also layers `http::trace`, which serves each request inside a `handler` span carrying the request and correlation ids, the method, and the matched route; a native host or test layers it with `axum::middleware::from_fn(trace)`. A delivery and a subcommand run inside the same span through the router's dispatch, which names the `topic` or the `command`.

Messaging is `topic("items.created", item_created)` on the same `Handlers`; topic matching is exact. A path or topic registered twice panics where the two meet, and `Handlers::into_router` is the router's only constructor. `topic` decodes JSON; `topic_with(topic, handler, decode)` takes a decoder `Fn(&Delivery) -> omnia_sdk::Result<I>` over the whole `Delivery` for any other wire format. `request` and `request_with` register a handler that answers a request (see [Handling incoming messages](#handling-incoming-messages)).

### Errors

An HTTP handler's error is anything `IntoResponse`. An `omnia_sdk::Error` becomes the JSON `ErrorBody` (`{"error","message"}`) at its kind's status, the same envelope the command transport writes to stderr; `?` on an `anyhow::Error` or a `serde_json::Error` converts into it. The macros `bad_request!`, `not_found!`, `conflict!`, `server_error!`, and `bad_gateway!` build an error of their kind under its default code; `Error::bad_request(code, description)` and its siblings name the code, for a specification's own reason code (`"segment_unknown"`). `Error::kind` answers the `ErrorKind` a caller matches on. A domain error with its own wire shape (another status, another content type such as an XML fault) implements `IntoResponse` itself and delegates its host failures to `omnia_sdk::Error::into_response`. A malformed body or query is refused by axum's extractor before the handler runs (`400`, or `415` without `Content-Type: application/json`).

## HTTP handlers

Register handlers on `Handlers` under axum's `get`/`post`/`put`/`patch`/`delete`, and export the router fn with `http::export!`:

```rust,noplayground
use axum::routing::get;
use omnia_sdk::{Handlers, Router, http};

fn router() -> Router {
    Handlers::new().route("/", get(echo_get).post(echo_post)).into_router()
}

http::export!(router);
```

The router fn may answer an `axum::Router` instead, for a guest that needs axum's own routing features.

For **outbound** HTTP requests inside a handler, start a request with `http::get`, `post`, `put`, `patch`, or `delete`, set its `header`, `bearer` token, `json` or `body`, and `timeout`, and end it with `fetch`, or `fetch_json` to require a 2xx status and decode a JSON reply. `omnia_sdk::http::fetch(request)` and `fetch_json(request)` take a standard `http::Request<Bytes>` built another way. `omnia_sdk::http::send` takes any body and fails with the host's own `ErrorCode`, as `examples/http-proxy` shows. A deployment may hold a guest to the host names its grants list, and a request to any other, or a redirect to one, fails as `ErrorCode::HttpRequestDenied` before any connection. With the default `http` feature, `omnia_sdk::http::cache::HttpCache` fetches through an `omnia:keyvalue` bucket. A request carrying `Cache-Control: max-age` and an `If-None-Match` entity-tag is then answered from the bucket while its stored response lives, as `examples/http-cache` shows.

> **Advanced: raw WASI HTTP export.** When you need to customize the `wasi:http/handler` export directly (as the standalone `examples/` do to show the low-level binding), implement `wasip3::exports::http::handler::Guest` and hand the request to `omnia_sdk::http::serve`, which opens the telemetry scope and the `handler` span itself:
>
> ```rust,noplayground
> struct HttpGuest;
> wasip3::http::service::export!(HttpGuest);
>
> impl Guest for HttpGuest {
>     async fn handle(request: Request) -> Result<Response, ErrorCode> {
>         let router = axum::Router::new().route("/", get(echo_get).post(echo_post));
>         omnia_sdk::http::serve(router, request).await
>     }
> }
> ```

## Using host interfaces

Each `omnia:*` interface is a module of `omnia-sdk`, and a module over resources the deployment declares has the same shape. A handler opens a resource with the module's one opener, `open` or `connect`, naming a binding the deployment declares. The types it holds are the generated ones, at the module root, and their helpers take `&str` and `&[u8]` and return a `Result`. A short verb over borrowed arguments is the everyday form, such as `Bucket::read`. The generated name is the full-control form, such as `Bucket::set` with its lifetime and precondition. A helper that encodes or decodes JSON ends in `_json`. The bindings `wit-bindgen` generates stay public under the module, such as `keyvalue::store` and `keyvalue::types`, for the raw calls. The guest never names a backend implementation — the host decides what backs each interface.

A binding fails with the shared `types::Error`, whose variant names the class of failure, such as `not-found`, `invalid`, or `unavailable`. `?` converts it into an `omnia_sdk::Error` under that code, every interface alike. The [provider contract](../reference/provider-contract.md#enforce-the-argument-rules) says which interfaces carry each code exactly. Absence is `None` rather than an error. An outcome the interface names for the caller to act on is the inner `Err` of a nested `Result<Result<T, Outcome>>`, such as a `keyvalue::Conflict` or a `model::Exhausted`. `?` on a `model`, `plugins`, or `vcs` outcome gives an `omnia_sdk::Error` under the outcome's name: a `BadRequest` for `model::Exhausted`, and a `Conflict` for the others. A module whose bindings a deployment names, such as `keyvalue` or `sql`, has a `DEFAULT` for the one a guest opens when it names no other, and `default()` opens it.

Key-value (`omnia:keyvalue`):

```rust,noplayground
use omnia_sdk::keyvalue::{self, Conflict};

let bucket = keyvalue::default().await?;
bucket.set_json("my_key", &body, Some(Duration::from_secs(300))).await?;
let res: Option<Body> = bucket.get_json("my_key").await?;
let visits = bucket.update_json("visits", None, |count: Option<u64>| Ok(count.unwrap_or(0) + 1)).await?;

let locks = keyvalue::open("locks").await?;
let lease = Some(Duration::from_secs(30));
if let Err(Conflict { current }) = locks.compare_and_swap("job", None, b"held", lease).await? {
    // another instance holds the lock
}
```

The deployment declares every bucket a guest opens, and opening an undeclared one fails with the code `not-found`. A key of `.`-separated segments, each holding only ASCII letters, digits, `_`, and `-`, works on every backend. A backend may reject any other key with the code `invalid`.

A write names what it does to the entry's lifetime, a `Lifetime` the host enforces:

- `Forever` and `TtlMs` replace it, as `set_json` does, and a `Duration` converts into the second.
- `Keep` and `KeepOrTtlMs` leave an existing entry the time it has left, as `compare_and_swap`, `set_json_if`, and `update_json` do.
- `add`, like the generated `increment`, gives its TTL only to a counter it creates.
- A zero TTL fails with the code `invalid`.

`read`, `write`, `remove`, and `contains` are the unconditional forms over raw bytes. The generated `get` reads a value with its `Revision`, and `set` with a `Precondition` writes only while it holds: the key is `Absent`, or its entry `Matches` a revision read before. A precondition that does not hold changes nothing and answers the entry found. `compare_and_swap` and `set_json_if` take the `Revision` read before, or `None` for an absent key, and answer the new one. `update_json` stores what its closure makes of the current value, and runs the closure again over the newer value when another write lands first.

Publishing a message (`omnia:messaging`), with a key that orders and partitions it:

```rust,noplayground
use omnia_sdk::messaging::{self, Message};

let client = messaging::default().await?;
let message = Message::json(&payload)?.with_key("customer-7");
client.send("my-topic", message).await?;
```

Every interface has a module, an example, and where it needs one a deep dive:

| Interface | Module (`omnia_sdk`) | Opener | Example | Deep dive |
| --------- | -------------------- | ------ | ------- | --------- |
| Key-value | `keyvalue` | `open`, `default` | `examples/keyvalue` | — |
| Messaging | `messaging` | `connect`, `default` | `examples/messaging` | [Messaging](messaging.md) |
| SQL | `sql` | `open`, `default` | `examples/sql` | [SQL](sql.md) |
| Document store | `docstore` | `open` | `examples/docstore` | [Document Store](document-store.md) |
| Blob store | `blobstore` | `open`, `create` | `examples/blobstore` | — |
| Secrets | `vault` | `open`, `default` | `examples/vault` | — |
| Config | `config` | `get`, `require` | `examples/config` | — |
| Identity/OAuth | `identity` | `open`, `default` | `examples/identity` | — |
| Outbound HTTP | `http` | `get`, `post`, `put`, `patch`, `delete`, `fetch` | `examples/http-proxy`, `examples/http-cache` | — |
| Model completions | `model` | `open`, `default`, `Request::complete` | `examples/model` | [Model Completions](model-completions.md) |
| Version control | `vcs` | `open`, `init`, `clone` | — | — |
| Dynamic plugins | `plugins` | `load` | — | — |
| WebSockets | `websocket` | `connect`, `default` | `examples/websocket` | [Messaging § WebSockets](messaging.md#websockets) |

### Configuration and the environment

A guest reads its settings through `config`, which the deployment's `omnia:config` backend answers. `ConfigDefault` serves the host variables `CONFIG_KEYS` names. The guest's own WASI environment, what `std::env` reads, holds less:

- `RUST_LOG`, the run's tracing filter (see [Tracing](#tracing));
- `NO_COLOR=1` when the host console is plain;
- each host variable the deployment's `[environment] inherit` names, or `run --env` adds.

A variable the deployment does not name never reaches the guest. See [Configuration](../reference/configuration.md#deployment-manifest-omniatoml).

## Handling incoming messages

A guest can export a messaging handler alongside (or instead of) an HTTP handler. Register the topics on a `Handlers`, close it into a router, and export the router fn with `messaging::export!`:

```rust,noplayground
use omnia_sdk::{Handlers, Router, messaging};

fn messaging_router() -> Router {
    Handlers::new().topic("items.created", on_item_created).into_router()
}

messaging::export!(messaging_router);
```

A handler that answers a request registers with `request`, which sends its output back as JSON, or with `request_with`, whose handler returns the reply `Message` (see [Messaging § Request-reply](messaging.md#request-reply)).

For guests that need direct control over the raw `omnia:messaging/incoming-handler` export (for example, `examples/messaging` demonstrating request-reply and fan-out, or a guest that sets a release delay), implement `omnia_sdk::exports::messaging::incoming_handler::Guest`, bind it with `omnia_sdk::exports::messaging::export!`, and dispatch through `omnia_sdk::messaging::handle`, which opens the guest telemetry scope around the delivery and settles a failure:

```rust,noplayground
use omnia_sdk::messaging::types::{Delivery, Settle};

pub struct Messaging;
omnia_sdk::exports::messaging::export!(Messaging);

impl omnia_sdk::exports::messaging::incoming_handler::Guest for Messaging {
    async fn handle(delivery: Delivery) -> Result<(), Settle> {
        omnia_sdk::messaging::handle(&router(), delivery).await
    }
}
```

`examples/messaging` demonstrates pub-sub, request-reply, and fan-out with the in-memory default backend; the same guest works against Kafka or NATS.

## Command-mode guests

For run-once workloads (jobs, CLIs, agent tasks), `omnia_sdk::command` is the command-line transport over the [handler contract](#the-handler-contract). A subcommand is one more handler on the same `Handlers` a module registers its paths and topics on: `command(name, handler)` adds it, the root merges the modules (`Handlers::from([..])` or `Handlers::merge`), and `into_router()` closes the whole into the `Router` that `run`s argv. The clap-backed parts (`Handlers::command`, the router's program methods and `run`, `parse`, `completions`, the `clap::ValueEnum` derive on `command::Format`) sit behind the `command` cargo feature (`omnia-sdk = { version = "...", features = ["command"] }`); the low-level `Command` projector needs no feature.

```rust,noplayground
use std::fmt;

use clap::Args;
use omnia_sdk::command::{self, Render};
use omnia_sdk::{Error, Handlers, Router};
use serde::Serialize;

/// Sync items from a source.
#[derive(Args)]
struct SyncInput {
    /// Where to sync from
    source: String,
}

#[derive(Serialize)]
struct Synced {
    count: usize,
}

impl Render for Synced {
    fn render(&self, out: &mut dyn fmt::Write) -> fmt::Result {
        writeln!(out, "synced {} items", self.count)
    }
}

async fn sync(input: SyncInput) -> Result<Synced, Error> {
    // ...
}

fn command_router() -> Router {
    Handlers::new()
        .command("sync", sync)
        .into_router()
        .hints(|error| (error.code() == "not_found").then(|| "run `app sync --help`".into()))
}

command::export!(command_router);
```

The pieces, in the order argv flows through them:

- **`Handlers::command(name, handler)`** registers a subcommand whose input implements `clap::Args` (`()` included) and whose output implements `Serialize` and `command::Render`; the input's doc comments are its help text, the struct's for the subcommand and each field's for its argument. **`command_with(subcommand, handler, decode)`** takes a `clap::Command` and a decoder `Fn(&ArgMatches) -> omnia_sdk::Result<I>` for any other grammar, as `topic_with` does for a wire format; a decoder's refusal is its error's envelope, so `bad_request!` answers a `bad_request` one rather than a clap usage error. The closed `Router` declares global `--format text|json` and `-v`/`--verbose` / `-q`/`--quiet` (`Verbosity`), answers `--help` and `--version` on stdout at exit 0, and renders clap usage errors on stderr at `USAGE_EXIT` (64, `EX_USAGE`). `Router::completions(shell)` (or `completions::<App>(shell, name)` for a custom grammar) produces a shell-completion script as a `Response`; `Shell` is re-exported from `clap_complete`.
- **The program** is the router: `--version` answers the guest crate's `CARGO_PKG_VERSION` and `--help` opens with its `CARGO_PKG_DESCRIPTION`, both stamped by `command::export!`; `Router::name` (default `argv[0]`), `version`, `about`, and `env_prefix` override them, and `Router::hints(|error| ..)` attaches a remedy hint to every failure.
- **`Command::new(&metadata, format)`** is the underlying projector the router dispatches through, and the escape hatch for a hand-written clap grammar: `parse::<App>(argv) -> Parsed` classifies argv, and `command.call(handler, input)` runs one verb, encoding a success body in the selected `command::Format` onto stdout at exit 0 — `Format::Text` is the output's `Render` form, `Format::Json` the output pretty-printed with a trailing newline.
- **`Response { stdout, stderr, exit }`** buffers both channels. `command::export!(command_router)` binds a router fn (or a zero-argument `async fn` returning `IntoExit`) as the `wasi:cli/run` export and writes the channels at that boundary through `IntoExit`: a `BrokenPipe` on either channel (a reader that went away) keeps the response's own exit, any other refused channel exits 3. Because the channels are whole buffers, a command that must stream writes to the process channels itself and returns `Result<(), u8>` or `()`, which the export also accepts; a guest that needs its own export writes the raw `wasip3` export and calls `omnia_sdk::command::execute` itself.
- **`Metadata::from_env(prefix)`** reads `<PREFIX>_REQUEST_ID`, `<PREFIX>_CORRELATION_ID`, `<PREFIX>_CAUSATION_ID`, `<PREFIX>_TENANT`, and `<PREFIX>_DEADLINE` from the environment (`Router::run` derives the default prefix from the program name, or `.env_prefix(..)` sets it; `run_with` takes explicit metadata). They reach the guest only when the deployment inherits them, as `[environment] inherit = ["<PREFIX>_*"]` or `run --env '<PREFIX>_*'` does. A missing request id is minted from the platform's CSPRNG (`wasi:random` in a guest), as on every transport, so a command invocation is as observable as an HTTP request: the router runs the verb in a `handler` span carrying the request and correlation ids and the `command`.

### Failures and the exit map

A decode or handler error becomes a `Failure` envelope on stderr, in the same `Format` as a success body. A handler's error takes its class and code from the first `omnia_sdk::Error`, binding error, or typed outcome in its source chain, as on a topic, and anything else is a `ServerError`. The process exits with `Error::exit_code()`:

| `ErrorKind` | `error` (the macros' default code) | Exit | HTTP status |
| ----------- | ---------------------------------- | ---- | ----------- |
| `BadRequest` | `bad_request` | 1 | 400 |
| `NotFound` | `not_found` | 2 | 404 |
| `ServerError` | `server_error` | 3 | 500 |
| `BadGateway` | `bad_gateway` | 4 | 502 |
| `Conflict` | `conflict` | 5 | 409 |
| clap usage error | — | 64 | — |

As text the envelope is `error[<code>]: <message>` followed by `hint: <hint>` when a hint is attached; as JSON it is flat: `{"error","message","exit-code","hint"?}` (the hint key is omitted when absent). `error` and `message` are the transport-neutral `ErrorBody` — `Error::into_response` emits the same two fields as an `application/json` body at its kind's status — so a client reads one discriminant whether it reached the handler over HTTP or a shell. Usage errors exit 64 rather than clap's default 2, so exit 2 always means a `NotFound` envelope.

- Arguments after `--` on the host command line arrive as the guest's argv (`args[0]` is the program name, supplied by the runtime).
- The host runtime must be built with `mode: command` — see [Composing a Runtime](composing-a-runtime.md).
- The telemetry lifecycle imports `omnia:otel`, so the deployment links `WasiOtel` (the no-op `OtelDefault` suffices).

`examples/cli` is the runnable pair: `greet`, `add`, `env`, and a `fail [CLASS]` verb that returns each error class so the exit map can be observed.

## Tracing

`tracing::debug!` and friends work inside guests, and spans flow to whatever OTel backend the host configures. Each transport's `export!` runs the export inside `omnia_sdk::otel::scope`: it initializes the guest's telemetry on entry and exports every span and metric as it returns. A span opened anywhere beneath it reaches the host. A root span takes the host's span for the call as its parent, so the guest's spans join the host's trace.

Annotate handlers with `#[omnia_sdk::otel::instrument]`, or `#[tracing::instrument]`. The export's scope already owns the telemetry lifecycle:

```rust
#[omnia_sdk::otel::instrument]
pub(crate) async fn place(Path(id): Path<String>) -> omnia_sdk::Result<Json<Option<Place>>> { /* ... */ }
```

A guest that binds its WASI export by hand annotates the export's `handle` with `#[omnia_sdk::otel::instrument]`, which opens that scope itself. `omnia_sdk::http::serve` and `omnia_sdk::messaging::handle` already do:

```rust
#[omnia_sdk::otel::instrument(name = "guest_handle", level = Level::INFO)]
async fn handle(request: Request) -> Result<Response, ErrorCode> { /* ... */ }
```

Only the outermost scope initializes and exports. A nested instrumented `async fn` adds its span to the same run, and on a sync function the attribute only opens a span.

The OpenTelemetry API works directly too: `global::tracer(..)` and `global::meter(..)` return Omnia's own providers, which record straight into the `omnia:otel` records the flush exports — no OpenTelemetry SDK is linked into the guest. Counters, histograms, and gauges export delta series (one instance per invocation); up-down counters are cumulative. Observable (callback) instruments are no-ops: record through a synchronous instrument instead.

Console output (events, never a span prefix) goes to stderr, so stdout stays the guest's own; it follows the guest's `RUST_LOG`, defaulting to `error`. Spans and metrics are exported whatever the filter admits: a span below the filter's level is never created, so an `#[instrument]` at `INFO` needs `RUST_LOG=info` to reach the host. A guest's `RUST_LOG` is the one its WASI environment carries: the runtime's tracing filter — a `-v`/`-q` flag's level, else the process `RUST_LOG`'s, else `info` for a command and `warn` for a server, with the process `RUST_LOG`'s targeted directives (`my_guest=debug`) on top (see [Verbosity flags](../reference/configuration.md#verbosity-flags)) — so every guest a runtime dispatches opens at the same level without naming another's, and an operator's `RUST_LOG=my_guest=trace` refines one guest under any flag. A command guest declares the flags by flattening `omnia_sdk::command::Verbosity` into its grammar; it never acts on them.

## Serving MCP tools

A guest can act as an [MCP](https://modelcontextprotocol.io) (Model Context Protocol) server — exposing tools and resources to AI agents over HTTP. Implement `omnia_sdk::mcp::McpServer` and serve `mcp::router` from your HTTP handler; see [Model Completions and MCP](model-completions.md#serving-mcp-tools-from-a-guest).
