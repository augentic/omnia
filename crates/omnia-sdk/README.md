# Omnia Guest

Shared traits, error types, and abstractions for building WASI guest components. This crate provides the glue between your business logic and the Omnia runtime capabilities.

## Quick Start

An HTTP route is a plain [axum](https://github.com/tokio-rs/axum) handler. It reaches its capabilities through the `Context<P>` extractor, generic over the provider `P` that serves them, reads its input with axum's `Path`, `Query`, and `Json` extractors, and answers a `Result` whose error converts to a wire response.

```rust,ignore
use axum::Json;
use axum::routing::post;
use omnia_sdk::api::{Client, Context};
use omnia_sdk::{Error, StateStore};
use serde::{Deserialize, Serialize};

#[derive(Deserialize)]
struct CreateItem {
    name: String,
}

#[derive(Debug, Serialize)]
struct ItemResponse {
    id: String,
    name: String,
}

async fn create_item<P: StateStore>(
    context: Context<P>, Json(input): Json<CreateItem>,
) -> Result<Json<ItemResponse>, Error> {
    let id = format!("item-{}", context.metadata.request_id.as_deref().unwrap_or("new"));
    context.state().set(&id, input.name.as_bytes(), None).await?;
    Ok(Json(ItemResponse { id, name: input.name }))
}

pub fn http_router<P: StateStore + 'static>(provider: P) -> axum::Router {
    axum::Router::new()
        .route("/items", post(create_item))
        .with_state(Client::new(provider))
}
```

Routes are registered with axum's own `get`/`post`/`put`/`patch`/`delete`, chained per path (`get(list).post(create)`). The router's state is one `Client`, which owns the provider; `Context<P>` extracts from it, so the capability accessors (`context.state()`, `context.config()`, ...) and the public `context.metadata` (the request's `x-request-id`, `x-correlation-id`, `x-causation-id`, and `x-tenant`, with a request id minted when absent) are available to every handler on the router. A handler bounds its `P` on the capability traits it reaches and nothing more; the `'static` axum needs is the router fn's (`Handlers<P>` carries it), so a handler never states it. `Context::new(provider, metadata)` builds one directly for unit-testing a fn that takes a context without a router. The guest's root crate binds the router fn to the WASI export over `WasiProvider`, the zero-sized provider that serves every capability through the WASI host:

```rust,ignore
omnia_sdk::api::http::export!(http_router);
```

Each transport module's `export!` (`http::export!`, `messaging::export!`, `command::export!`) binds one WASI export and expands to nothing on native targets, so the same crate's handler tests run natively over `omnia_test::guest::Provider`, driving the router with `omnia_test::guest::send` (one request, the reply with its body collected) and `omnia_test::guest::deliver` (one delivery on a topic). On `wasm32` the HTTP export calls the router fn with `WasiProvider` (or a custom provider passed as a second argument) and serves each request inside a `handler` tracing span carrying the request and correlation ids, the method, and the matched route (`api::http::trace`, an axum middleware a native test can layer on too). Omnia creates one WASI component instance per HTTP request; the router fn runs inside each `handle` call, and axum's route-state clones share that client's `Arc<P>` only for that request. Durable application state belongs in host-side capabilities, not guest statics.

### Errors

A handler's error is anything `IntoResponse`. `omnia_sdk::Error` (`bad_request!`, `not_found!`, `server_error!`, `bad_gateway!`, or `Error::bad_request(code, description)` and its siblings when the specification names the `error` code a client reads) answers the JSON `api::ErrorBody` (`{"error","message"}`) at the variant's status, the same envelope the command transport writes to stderr, and `?` on an `anyhow::Error` or a `serde_json::Error` converts into it. A domain error with its own wire shape (another status, another content type) implements `IntoResponse` itself and delegates its host-failure variant to `omnia_sdk::Error::into_response`.

### Messaging

Messaging routes are `Handler`s: any `async fn(Context<P>, I) -> Result<O, E>`, the context first as in an HTTP handler, registered on exact topics and exported the same way. Every fn of that shape (and every `Clone` closure of it, so a closure can carry configuration) is a `Handler<P, I>` through a blanket impl.

```rust,ignore
use omnia_sdk::api::{Client, Context, Handlers, Router, messaging};

async fn item_created<P: StateStore>(context: Context<P>, input: CreateItem) -> Result<(), Error> {
    context.state().set(&input.name, b"created", None).await
}

pub fn messaging_router<P: StateStore + 'static>(provider: P) -> Router<P> {
    Handlers::new().topic("items.created", item_created).with_state(Client::new(provider))
}

messaging::export!(messaging_router);
```

`topic(topic, handler)` decodes JSON and acknowledges successful output; `topic_with(topic, handler, decode)` takes a decoder `Fn(&Delivery) -> Result<I, DecodeError>` over the whole delivery for any other wire format. A module registers its HTTP routes, messaging topics, and subcommands on a `Handlers<P>` of its own, with no client, and the root merges each (`Handlers::from([..])` or `Handlers::merge`) and closes the whole into the one `Router<P>` with `Handlers::with_state`, the router's only constructor. A fn generic over the whole provider, such as a module's `handlers<P>()`, takes `omnia_sdk::Capabilities` as its bound in place of listing every trait. A delivery and a subcommand run inside the same `handler` span through the router's dispatch, which names the `topic` or the `command`.

### Command-mode guests

A subcommand is one more handler on the same `Handlers<P>` (`command` feature): `command(name, handler)` registers it beside the paths and topics, where the handler's input implements `clap::Args` (`()` included) and its output `Serialize` and `command::Render`, and `command_with(subcommand, handler, decode)` takes a `clap::Command` with a decoder `Fn(&ArgMatches) -> Result<I, DecodeError>` for any other grammar. The same `with_state(Client::new(provider))` closes the whole into the `Router<P>` that `run`s argv, and `command::export!(command_router)` binds the router fn as the `wasi:cli/run` export (or a zero-argument `async fn` returning `IntoExit`), writing the channels at that boundary. Omnia creates a fresh component instance for each command invocation.

```rust,ignore
use clap::Args;
use omnia_sdk::api::{Client, Context, Handlers, Router, command};

/// Create an item by name.
#[derive(Args)]
struct CreateItem {
    /// The item's name
    name: String,
}

async fn create<P: StateStore>(context: Context<P>, input: CreateItem) -> omnia_sdk::Result<()> {
    // ...
}

pub fn command_router<P: StateStore + 'static>(provider: P) -> Router<P> {
    Handlers::new().command("create", create).with_state(Client::new(provider))
}

command::export!(command_router);
```

The input type's doc comments are the help text (the struct's for the subcommand, each field's for its argument). A success body is encoded in the selected `command::Format` (`--format text|json`; `Text` through the output's `Render` impl, `Json` pretty-printed) onto stdout at exit 0. A decode or handler error is a `Failure` envelope on stderr at `Error::exit_code()` — `BadRequest` 1, `NotFound` 2, `ServerError` 3, `BadGateway` 4 — rendered as `error[<code>]: <message>` (plus `hint: …` when `Router::hints(..)` or `with_hint` attached one) or as flat JSON `{"error","message","exit-code","hint"?}`; `error` and `message` are the same `ErrorBody` HTTP emits. A clap usage error exits `USAGE_EXIT` (64), so exit 2 always means `NotFound`. `Metadata::from_env(prefix)` reads `<PREFIX>_REQUEST_ID` / `_CORRELATION_ID` / `_CAUSATION_ID` / `_TENANT`, minting a request id when absent as every transport does.

The router is also the program: `--version` answers the guest crate's `CARGO_PKG_VERSION` and `--help` opens with its `CARGO_PKG_DESCRIPTION`, both stamped by `command::export!`, and `Router::name`, `version`, `about`, and `env_prefix` override them. The runtime's verbosity flags — `-v`/`--verbose` and `-q`/`--quiet`, each repetition one level from the command's default of `info` — are read by the host out of argv and set the bare level of the guest's `RUST_LOG` before it runs (the process `RUST_LOG`'s targeted directives stay on top); the router declares them (along with `--format`) itself, and a hand-written clap grammar declares them with `#[command(flatten)] verbosity: omnia_sdk::api::command::Verbosity`.

## Capabilities

The guest crate exposes trait-based abstractions for host capabilities. `WasiProvider` implements every one of them on `wasm32` by calling the WASI host; a handler bounds its `P` on the traits it uses and never names the provider. `Capabilities` is every trait at once (plus `Send + Sync + 'static`), the bound for a fn generic over the whole provider rather than over the capabilities one handler reaches.

| Trait | Purpose |
| ----- | ------- |
| `Config` | Read configuration values from the host. |
| `HttpRequest` | Make outbound HTTP requests. |
| `Publish` | Publish messages to a topic. |
| `StateStore` | Key-value state in the deployment's declared buckets: `get`, `set` with an optional TTL the host enforces, `delete`, a one-shot `compare_and_swap` whose mismatch answers the `keyvalue::Conflict` it found, and atomic `increment`. A provider's own handle uses the `default` bucket, and `named(name)` selects another. |
| `Identity` | Obtain access tokens from an identity provider. |
| `TableStore` | Execute parameterized SQL queries and statements over `wasi:sql`. |
| `Broadcast` | Send events over WebSocket channels. |
| `Plugins` | Request guest loads through `omnia:plugins/loader`: name a `Location` — one of the deployment's declared guests, a component path beneath one of its read-only mounts, or a registry package — with an optional `Digest` pin, and receive a typed `Plugin` handle once it is active. The `plugins` module carries the `Location` and `Digest` types and typed refusals convertible into `Error`. |
| `BlobStore` | Object storage: the ten primitives an implementor writes plus five derived operations (`has`, `delete_objects`, `clear`, `copy_object`, `move_object`) — one host call each on `WasiProvider`, composed from the primitives by default. |
| `DocumentStore` | Document CRUD and filtered queries. |
| `Vault` | Read, write, delete, check, and list secrets in the vault locker. |
| `Vcs` | Version-control operations over lent repository and working-copy mounts. |
| `Model` | Prompt completions, with tool calls answered by a guest closure. A request with `check` set has the guest judge each candidate answer (`Ok` accepts, `Err(text)` is the correction the backend sends back to the model); `model::Question<T>` runs that exchange for a `Deserialize + JsonSchema` answer type and returns the accepted `T`. |

Every trait is annotated with `#[omnia_guest_macros::capability]`, which emits a companion `Has<Capability>` accessor trait with a blanket forwarding impl and implementations for `Arc<T>`, `&T`, and `Box<T>` where `T` implements the capability. `#[derive(omnia_sdk::Provider)]` implements those `Has*` traits for each `#[provider(...)]`-annotated field. Each `Has*` trait names the type it delegates to as `Inner`, so a capability's associated types, such as `StateStore::Named`, are the field's own. A `pub` provider therefore needs `pub` field types.

### Example: Using Capabilities

```rust,ignore
use omnia_sdk::{StateStore, Publish, Message};

async fn process(provider: &impl StateStore + Publish) -> anyhow::Result<()> {
    // Store some state
    provider.set("last_run", b"now", None).await?;

    // Publish a message
    let msg = Message::new(b"job_completed");
    provider.send("jobs.events", &msg).await?;

    Ok(())
}
```

## Error Handling

The crate provides an `Error` enum with four variants (`BadRequest`, `NotFound`, `ServerError`, `BadGateway`), each mapped to an HTTP status (`Error::status()`) and a process exit code (`Error::exit_code()`: 1 / 2 / 3 / 4). Every transport reports a failure as the same `api::ErrorBody { error, message }` (`code()` and `description()`). The macros `bad_request!`, `not_found!`, `server_error!`, and `bad_gateway!` build a variant under its default code (`bad_request`, ...); the constructors `Error::bad_request(code, description)` and siblings name the code, for a specification's own reason codes.

```rust,ignore
use omnia_sdk::{Error, bad_request};

fn validate(name: &str) -> Result<(), Error> {
    if name.is_empty() {
        return Err(bad_request!("name cannot be empty"));
    }
    if name.len() > 64 {
        return Err(Error::bad_request("name_too_long", format!("{} chars, 64 allowed", name.len())));
    }
    Ok(())
}
```

A capability that names its failure classes returns a `capability::Error`, as `StateStore` does. Its `ErrorKind` is one of `NotFound`, `Denied`, `Invalid`, `Unsupported`, `Unavailable`, or `Other`, and a caller matches on `kind()` rather than the message. `?` converts it into `Error`: `Unavailable` becomes a `BadGateway` and every other kind a `ServerError`, through any `anyhow` context wrapped around it.

## Architecture

See the [workspace documentation](https://github.com/augentic/omnia) for the full architecture guide.

## Cargo features

- `sql` *(default)*: the `TableStore` capability over `wasi:sql`.
- `docstore` *(default)*: the `DocumentStore` capability and the `docstore` value types.
- `http` *(default)*: the axum-backed `api::http` handler support (the `Context` and `Metadata` extractors, `trace`, `Error: IntoResponse`, and `http::export!`) and the `mcp` server. The outbound `HttpRequest` capability and `Error::status()` need no feature.
- `command`: the clap-backed parts of `api::command` (`Handlers::command`, the `Router`'s program methods and `run`, `parse`, `completions`, `Response::usage`, the `clap_complete::Shell` re-export, and the `clap::ValueEnum` derive on `command::Format`). The `Command` projector, `Response`, `Failure`, `command::Format`, `command::Render`, and `command::export!` need no feature.

Guests that do not use SQL, documents, or inbound HTTP can disable defaults to shrink wasm build time and size, naming only the capabilities they use (`features = ["sql"]` for `TableStore` alone); a command-only guest is `default-features = false, features = ["command"]`.

## License

MIT OR Apache-2.0
