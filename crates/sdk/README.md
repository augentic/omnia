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

async fn create_item<P: StateStore + 'static>(
    context: Context<P>, Json(input): Json<CreateItem>,
) -> Result<Json<ItemResponse>, Error> {
    let id = format!("item-{}", context.metadata.request_id.as_deref().unwrap_or("new"));
    context.provider().set(&id, input.name.as_bytes(), None).await?;
    Ok(Json(ItemResponse { id, name: input.name }))
}

pub fn http_router<P: StateStore + 'static>(provider: P) -> axum::Router {
    axum::Router::new()
        .route("/items", post(create_item))
        .with_state(Client::new(provider))
}
```

Routes are registered with axum's own `get`/`post`/`put`/`patch`/`delete`, chained per path (`get(list).post(create)`). The router's state is one `Client`, which owns the provider; `Context<P>` extracts from it, so `context.provider()` and the public `context.metadata` (the request's `x-request-id`, `x-correlation-id`, `x-causation-id`, and `x-tenant`, with a request id minted when absent) are available to every handler on the router. `Context::new(provider, metadata)` builds one directly for unit-testing a fn that takes a context without a router. The guest's root crate binds the router fn to the WASI export over `WasiProvider`, the zero-sized provider that serves every capability through the WASI host:

```rust,ignore
omnia_sdk::api::http::export!(http_router);
```

Each transport module's `export!` (`http::export!`, `messaging::export!`, `command::export!`) binds one WASI export and expands to nothing on native targets, so the same crate's handler tests run natively over `omnia_test::guest::Provider`, driving the router with `tower::ServiceExt::oneshot`. On `wasm32` the HTTP export calls the router fn with `WasiProvider` (or a custom provider passed as a second argument) and serves each request inside a `handler` tracing span carrying the request and correlation ids (`api::http::trace`, an axum middleware a native test can layer on too). Omnia creates one WASI component instance per HTTP request; the router fn runs inside each `handle` call, and axum's route-state clones share that client's `Arc<P>` only for that request. Durable application state belongs in host-side capabilities, not guest statics.

### Errors

A handler's error is anything `IntoResponse`. `omnia_sdk::Error` (`bad_request!`, `not_found!`, `server_error!`, `bad_gateway!`) answers the JSON `api::ErrorBody` (`{"error","message"}`) at the variant's status, the same envelope the command transport writes to stderr, and `?` on an `anyhow::Error` or a `serde_json::Error` converts into it. A domain error with its own wire shape (another status, another content type) implements `IntoResponse` itself and delegates its host-failure variant to `omnia_sdk::Error::into_response`.

### Messaging

Messaging routes are `Handler`s: any `async fn(I, Context<P>) -> Result<O, E>`, registered on exact topics and exported the same way. Every fn of that shape (and every `Clone` closure of it, so a closure can carry configuration) is a `Handler<P, I>` through a blanket impl.

```rust,ignore
use omnia_sdk::api::messaging::{self, Router};

async fn item_created<P: StateStore>(input: CreateItem, context: Context<P>) -> Result<(), Error> {
    context.provider().set(&input.name, b"created", None).await
}

pub async fn messaging_router<P: StateStore + Send + Sync + 'static>(provider: P) -> Router<P> {
    Router::new(Client::new(provider)).route("items.created", item_created)
}

messaging::export!(messaging_router);
```

`route(topic, handler)` decodes JSON and acknowledges successful output; `route_with(topic, handler, decode)` takes a decoder `Fn(&Delivery) -> Result<I, DecodeError>` over the whole delivery for any other wire format. A module that owns topics registers them on a `messaging::Routes` of its own, with no client, and the root merges each (`Routes::from([..])` or `Router::merge`) and closes the whole into the one `Router` with `Routes::with_state`, as `http::Routes<P>` is merged (`http::Routes::from([..])`) and closed into `http::Router` by `with_state`; `Routes::map` rewrites every topic once, for an environment prefix or a namespace. `Client::call(handler, input, &metadata)` invokes a `Handler` directly inside the same `handler` span.

### Command-mode guests

`api::command` is the command-line transport over `Handler`s (`command` feature for the clap-backed parts): `parse::<App>(argv)` classifies argv into the grammar or one of clap's own responses (`Parsed::{App, Display, Usage}`), and `Command::new(&client, &metadata, format).call(handler, decode, render)` projects one verb — decode → `Client::call` → encode — onto a `Response { stdout, stderr, exit }`. `command::export!(main)` binds an `async fn main() -> Response` as the `wasi:cli/run` export and writes the channels at that boundary (`IntoExit`; `Result<(), u8>` and `()` entries are accepted too). Omnia creates a fresh component instance for each command invocation.

```rust,ignore
use omnia_sdk::api::command::{self, Command, Parsed, Response, parse};
use omnia_sdk::api::{Client, Metadata};

command::export!(main);

async fn main() -> Response {
    let app = match parse::<App>(wasip3::cli::environment::get_arguments()) {
        Parsed::App(app) => app,
        Parsed::Display(text) => return Response::success(text),
        Parsed::Usage(error) => return Response::usage(&error),
    };
    let client = Client::new(omnia_sdk::WasiProvider);
    let metadata = Metadata::from_env("APP");
    let command = Command::new(&client, &metadata, app.format);
    match app.verb {
        Verb::Created(input) => command.call(item_created, || Ok(input), render_unit).await,
    }
}
```

A success body is encoded in the selected `api::Format` (`Text` through the `render` closure, `Json` pretty-printed) onto stdout at exit 0. A decode or handler error is a `Failure` envelope on stderr at `Error::exit_code()` — `BadRequest` 1, `NotFound` 2, `ServerError` 3, `BadGateway` 4 — rendered as `error[<code>]: <message>` (plus `hint: …` when `.hints(..)` or `with_hint` attached one) or as flat JSON `{"error","message","exit-code","hint"?}`; `error` and `message` are the same `ErrorBody` HTTP emits. A clap usage error exits `USAGE_EXIT` (64), so exit 2 always means `NotFound`. `Metadata::from_env(prefix)` reads `<PREFIX>_REQUEST_ID` / `_CORRELATION_ID` / `_CAUSATION_ID` / `_TENANT`, minting a request id when absent as every transport does.

The runtime's verbosity flags — `-v`/`--verbose` and `-q`/`--quiet`, each repetition one level from the command's default of `info` — are read by the host out of argv and set the bare level of the guest's `RUST_LOG` before it runs (the process `RUST_LOG`'s targeted directives stay on top); argv still arrives verbatim, so a grammar declares them with `#[command(flatten)] verbosity: omnia_sdk::api::command::Verbosity` to list them in help and completions and to refuse `-v` beside `-q`. The command never acts on the parsed values.

## Capabilities

The guest crate exposes trait-based abstractions for host capabilities. `WasiProvider` implements every one of them on `wasm32` by calling the WASI host; a handler bounds its `P` on the traits it uses and never names the provider.

| Trait | Purpose |
| ----- | ------- |
| `Config` | Read configuration values from the host. |
| `HttpRequest` | Make outbound HTTP requests. |
| `Publish` | Publish messages to a topic. |
| `StateStore` | Get/set/delete key-value state with optional TTL, plus one-shot `cas` (conflict is a typed `CasError::Conflict`) and atomic `increment`. |
| `Identity` | Obtain access tokens from an identity provider. |
| `TableStore` | Execute parameterized SQL queries and statements over `wasi:sql`. |
| `Broadcast` | Send events over WebSocket channels. |
| `Plugins` | Request guest loads through `omnia:plugins/loader`: name a `Location` — one of the deployment's declared guests, a component path beneath one of its read-only mounts, or a registry package — with an optional `Digest` pin, and receive a typed `Plugin` handle once it is active. The `plugins` module carries the `Location` and `Digest` types and typed refusals convertible into `Error`. |
| `BlobStore` | Object storage: the ten primitives an implementor writes plus five derived operations (`has`, `delete_objects`, `clear`, `copy_object`, `move_object`) — one host call each on `WasiProvider`, composed from the primitives by default. |
| `DocumentStore` | Document CRUD and filtered queries. |
| `Vault` | Read, write, delete, check, and list secrets in the vault locker. |
| `Vcs` | Version-control operations over lent repository and working-copy mounts. |
| `Model` | Prompt completions, with tool calls answered by a guest closure. A request with `check` set has the guest judge each candidate answer (`Ok` accepts, `Err(text)` is the correction the backend sends back to the model); `model::Question<T>` runs that exchange for a `Deserialize + JsonSchema` answer type and returns the accepted `T`. |

Every trait is annotated with `#[omnia_guest_macros::capability]`, which emits a companion `Has<Capability>` accessor trait with a blanket forwarding impl and implementations for `Arc<T>`, `&T`, and `Box<T>` where `T` implements the capability. `#[derive(omnia_sdk::Provider)]` implements those `Has*` traits for each `#[provider(...)]`-annotated field.

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

The crate provides an `Error` enum with four variants (`BadRequest`, `NotFound`, `ServerError`, `BadGateway`), each mapped to an HTTP status (`Error::status()`) and a process exit code (`Error::exit_code()`: 1 / 2 / 3 / 4), and helper macros for ergonomic error creation. Every transport reports a failure as the same `api::ErrorBody { error, message }` (`code()` and `description()`).

```rust,ignore
use omnia_sdk::{bad_request, server_error, not_found};

fn validate(name: &str) -> Result<(), omnia_sdk::Error> {
    if name.is_empty() {
        return Err(bad_request!("name cannot be empty"));
    }
    Ok(())
}
```

## Architecture

See the [workspace documentation](https://github.com/augentic/omnia) for the full architecture guide.

## Cargo features

- `sql` *(default)*: the `TableStore` capability over `wasi:sql`.
- `docstore` *(default)*: the `DocumentStore` capability and the `docstore` re-exports.
- `http` *(default)*: the axum-backed `api::http` handler support (the `Context` and `Metadata` extractors, `trace`, `Error: IntoResponse`, and `http::export!`) and the `mcp` server. The outbound `HttpRequest` capability and `Error::status()` need no feature.
- `command`: the clap-backed parts of `api::command` (`parse`, `completions`, `Response::usage`, the `clap_complete::Shell` re-export, and the `clap::ValueEnum` derive on `api::Format`). The `Command` projector, `Response`, `Failure`, and `command::export!` need no feature.

Guests that do not use SQL, documents, or inbound HTTP can disable defaults to shrink wasm build time and size, naming only the capabilities they use (`features = ["sql"]` for `TableStore` alone); a command-only guest is `default-features = false, features = ["command"]`.

## License

MIT OR Apache-2.0
