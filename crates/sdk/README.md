# omnia-sdk

`omnia-sdk` is the SDK for writing an Omnia guest, a WASI component that serves HTTP requests, messaging deliveries, and command lines. It carries the generated bindings for every `omnia:*` interface, typed helpers over them, and a router that binds a guest's handlers to its exports.

## Quick start

A handler is an `async fn` that takes a `Context`. It reaches the host through an interface module, here `keyvalue`. Register it on `Handlers`, close the handlers with `into_router()`, and bind the router fn with `http::export!`.

```rust
use axum::Json;
use axum::routing::post;
use omnia_sdk::{Context, Handlers, Router, http, keyvalue};
use serde::{Deserialize, Serialize};

#[derive(Deserialize, Serialize)]
struct Item {
    id: String,
    name: String,
}

async fn create_item(_context: Context, Json(item): Json<Item>) -> omnia_sdk::Result<Json<Item>> {
    let bucket = keyvalue::open(keyvalue::DEFAULT).await?;
    bucket.set_json(&item.id, &item, None).await?;
    Ok(Json(item))
}

fn router() -> Router {
    Handlers::new().route("/items", post(create_item)).into_router()
}

http::export!(router);
```

[Writing guests](https://github.com/augentic/omnia/blob/main/docs/guides/writing-guests.md) walks through a whole guest.

## Interfaces

Each `omnia:*` interface is one module. A module over resources the deployment declares, such as `keyvalue`, `sql`, or `messaging`, has the same shape:

- One opener, `open` or `connect`, takes the name of a binding the deployment declares. A module with a default binding, such as `keyvalue` or `sql`, has a `DEFAULT` naming it.
- The types a guest holds are the generated ones, at the module root, such as `keyvalue::Bucket` and `sql::Database`.
- Helpers are methods on those types. They take `&str` and `&[u8]` and return a `Result`.
- The bindings `wit-bindgen` generates stay public as submodules, such as `keyvalue::store` and `keyvalue::types`, for the raw calls.

Absence is `Ok(None)`, not an error, and `?` converts a binding's error into an `Error` under its code. An outcome the interface names for the caller to act on, such as a `compare_and_swap` that lost its race, is the inner `Err` of a nested `Result<Result<T, Outcome>>`.

| Module | Opener | Helpers |
| ------ | ------ | ------- |
| `blobstore` | `open`, `admin::create_container` | Whole-object `Container::read`, `read_range`, and `write`. |
| `config` | `get` | Typed `require` and `parse`, and `#[derive(Settings)]`. |
| `docstore` | `open` | Typed `Collection::get_as`, `insert`, `upsert`, and `remove`, and `find` over a `Query`. |
| `http` | `fetch`, `send` | `fetch_json` for a JSON reply, and `cache::HttpCache` for responses cached in a keyvalue bucket, behind the `http-cache` feature. |
| `identity` | `open` | `Identity::token`, an `AccessToken` with the instant it expires. |
| `keyvalue` | `open` | `Bucket::read`, `write`, `remove`, and `keys`, JSON `get_json` and `set_json`, and `compare_and_swap`. |
| `messaging` | `connect` | `Client::send`, `request`, and `reply`, over a `Message::new` or `Message::json`. |
| `model` | `open`, `Request::complete` | `complete_with` for tool calls, `complete_checked` for a checked answer, `Message::user` and its siblings, and `Question` for a typed answer. |
| `plugins` | `load` | `Source::name`, over the generated `Source` and `Plugin`. |
| `sql` | `open` | `query_as` and `query_opt` through `FromRow`, `exec`, and `Database::begin` for a transaction. |
| `vault` | `open` | None: a locker's generated methods. |
| `vcs` | `open`, `init`, `clone` | `Workspace::add`, `fetch`, `push`, and `remote_bookmark`, and `log_all` and `status_all` over every page. |
| `websocket` | `connect` | `Client::broadcast`, `send`, `close`, and `exists`. |

The unstable `blobstore::uploads` and `presign`, `messaging::consumer`, `log`, and `admin`, and `otel::logs` are raw bindings with no helpers. `otel` holds the guest's telemetry library, and `#[omnia_sdk::otel::instrument]` opens a span around a fn. `mcp` serves the Model Context Protocol from a guest, behind the `http` feature.

## Transports

HTTP routes, messaging topics, and subcommands all register on a `Handlers`. A module registers its own on one, and the guest merges them with `Handlers::from([..])` or `merge` before `into_router` closes the whole into one `Router`. Two handlers for one path and method, one topic, or one subcommand are refused when they meet. `Context`, `Handlers`, `Router`, and `Error` sit at the crate root.

Each transport's `export!` binds the router fn to one WASI export: `http::export!`, `messaging::export!`, or `command::export!`. On native targets it binds nothing, and a test or a native host serves the router itself.

### HTTP

An HTTP handler is a plain [axum](https://github.com/tokio-rs/axum) handler. It takes `Context` as an extractor beside axum's own, such as `Path`, `Query`, and `Json`. The `http` module documents the rest.

- `Handlers::route(path, method_router)` binds axum's `get`, `post`, `put`, `patch`, and `delete`, chained per path.
- `context.metadata` carries the `x-request-id`, `x-correlation-id`, `x-causation-id`, `x-tenant`, and `x-deadline` headers. A missing request id is minted.
- `http::export!` binds the `wasi:http` export, and serves each request inside the `handler` span `http::trace` opens.
- Omnia creates one component instance per request, so durable state belongs in a host interface, not a static.

```rust,ignore
use axum::Json;
use axum::extract::Path;
use axum::routing::get;
use omnia_sdk::{Context, Handlers, keyvalue, not_found};

async fn get_item(_context: Context, Path(id): Path<String>) -> omnia_sdk::Result<Json<Item>> {
    let bucket = keyvalue::open(keyvalue::DEFAULT).await?;
    let item = bucket.get_json(&id).await?;
    item.map(Json).ok_or_else(|| not_found!("no item {id}"))
}

pub fn handlers() -> Handlers {
    Handlers::new().route("/items/{id}", get(get_item))
}
```

### Messaging

A messaging handler is any `async fn(Context, I) -> Result<O, E>`, the context first as in an HTTP handler. A `Clone` closure of that shape is one too, so a closure can carry configuration. The `messaging` module documents the rest.

- `Handlers::topic(topic, handler)` decodes the payload as JSON.
- `Handlers::topic_with(topic, handler, decode)` takes a decoder over the whole `Delivery`, for any other wire format. A handler that answers a request registers here, so its decoder can pass the delivery's `reply_to` to `Client::reply`.
- A topic matches exactly, with no wildcards.
- `messaging::export!` binds the `omnia:messaging` handler.
- A guest that settles every delivery itself, or sets a release delay, implements `exports::messaging` in place of a router.

```rust,ignore
use omnia_sdk::{Context, Handlers, Router, keyvalue, messaging};

async fn item_created(_context: Context, item: Item) -> omnia_sdk::Result<()> {
    let bucket = keyvalue::open(keyvalue::DEFAULT).await?;
    bucket.set_json(&item.id, &item, None).await
}

fn router() -> Router {
    Handlers::new().topic("items.created", item_created).into_router()
}

messaging::export!(router);
```

### Command

A subcommand is one more handler on `Handlers`, behind the `command` feature. Its input implements `clap::Args`, whose doc comments are the help text. Its output implements `Serialize` and `command::Render`. The `command` module documents the rest.

- `Handlers::command(name, handler)` registers a subcommand.
- `Handlers::command_with(command, handler, decode)` takes a `clap::Command` and a decoder over its `ArgMatches`, for any other grammar.
- A success goes to stdout at exit 0, in the format `--format text|json` selects.
- `--version` and `--help` answer the guest crate's version and description. `Router::version` and `Router::about` override them.
- `-v` and `-q` set the guest's log level, which the host reads from argv.
- `command::export!` binds the `wasi:cli/run` export, from a router fn or a zero-argument `async fn`. Omnia creates one component instance per invocation.

```rust,ignore
use clap::Args;
use omnia_sdk::{Context, Handlers, Router, command, keyvalue};

/// Create an item.
#[derive(Args)]
struct Create {
    /// The item's id.
    id: String,

    /// The item's name.
    name: String,
}

async fn create(_context: Context, input: Create) -> omnia_sdk::Result<String> {
    let item = Item { id: input.id, name: input.name };
    keyvalue::open(keyvalue::DEFAULT).await?.set_json(&item.id, &item, None).await?;
    Ok(format!("created {}", item.id))
}

fn router() -> Router {
    Handlers::new().command("create", create).into_router()
}

command::export!(router);
```

## Errors

`omnia_sdk::Error` has four variants:

| Variant | HTTP status | Exit code | Default code |
| ------- | ----------- | --------- | ------------ |
| `BadRequest` | 400 | 1 | `bad_request` |
| `NotFound` | 404 | 2 | `not_found` |
| `ServerError` | 500 | 3 | `server_error` |
| `BadGateway` | 502 | 4 | `bad_gateway` |

The macros `bad_request!`, `not_found!`, `server_error!`, and `bad_gateway!` build a variant under its default code from a format string. `Error::bad_request(code, description)` and its siblings name the code instead, for a specification's own reason codes. `?` converts an `anyhow::Error` or a `serde_json::Error` into an `Error`.

Each transport reports an `Error` by its code and description:

- HTTP answers `{"error": "<code>", "message": "<description>"}` at the variant's status.
- A command writes the same body to stderr, as text or JSON, at the variant's exit code. A clap usage error exits 64, so exit 2 always means `NotFound`.
- A failed delivery returns the error to the messaging host.

An HTTP handler's error is anything `IntoResponse`, so a domain error with a wire shape of its own implements it. A topic or subcommand handler fails with any error type. The first `Error`, binding error, or typed outcome in its source chain sets the variant and code, and any other error is a `ServerError` with the code `server_error`.

A binding's error converts into an `Error` whose code names the class of failure:

- `not-found`, `denied`, `invalid`, `unsupported`, or `other` on a `ServerError`.
- `unavailable` on a `BadGateway`, when the upstream did not answer or the call exceeded its time bound.

The [provider contract](https://github.com/augentic/omnia/blob/main/docs/reference/provider-contract.md#enforce-the-argument-rules) says which interfaces carry each code exactly. An error keeps its variant and code through any `anyhow` context around it.

A typed outcome converts into a `BadRequest` under its own name, for a handler that passes it on rather than acting on it:

- `model::Exhausted` as `exhausted`;
- `plugins::Conflict` as `conflict`;
- `vcs::Occupied` as `occupied`, and `vcs::Diverged` as `diverged`;
- `vcs::Unmerged` as `conflicts` or `pending`.

## Testing

A guest's host calls panic off `wasm32`, so a handler that reaches the host is tested as the compiled component:

- `omnia_test::host` runs the component through omnia's own runtime, over in-memory backends or scripted ones.
- `omnia_test::host::send` and `deliver` drive one request or one delivery through it in process.
- A handler that reaches no host interface also runs natively, through `Router::handle`, `Router::run`, or the `axum::Router` a `Router` converts into.

[Testing Omnia code](https://github.com/augentic/omnia/blob/main/docs/guides/testing-omnia-code.md) walks through each rung.

## Cargo features

- `http` *(default)*: the axum-backed inbound handlers (`http::export!`, `serve`, and `trace`), `Error: IntoResponse`, and the `mcp` server. Outbound `http::fetch` needs no feature.
- `command`: the clap-backed `command` program, with `Handlers::command`, `Router::run`, and shell completions. `command::export!` and the `Response` and `Failure` types need no feature.
- `http-cache`: `http::cache`, an outbound response cache over a keyvalue bucket, driven by each request's `Cache-Control` and `If-None-Match`. It needs neither `http` nor `command`.

Every interface is always built. A guest that serves no inbound HTTP can disable the default features to shrink its build. A command-only guest is `default-features = false, features = ["command"]`.

## License

MIT OR Apache-2.0
