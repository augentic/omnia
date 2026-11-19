# omnia-sdk

`omnia-sdk` is the SDK for writing an Omnia guest, a WASI component that serves HTTP requests, messaging deliveries, and command lines. It carries the generated bindings for every `omnia:*` interface, typed helpers over them, and a router that binds a guest's handlers to its exports.

## Quick start

A handler is an `async fn` that takes a `Context`. It reaches the host through an interface module, here `keyvalue`. Register it on `Handlers`, close the handlers with `into_router()`, and bind the router fn with `http::export!`.

```rust
use axum::Json;
use axum::routing::post;
use omnia_sdk::api::{Context, Handlers, Router, http};
use omnia_sdk::keyvalue::{DEFAULT, store};
use serde::{Deserialize, Serialize};

#[derive(Deserialize, Serialize)]
struct Item {
    id: String,
    name: String,
}

async fn create_item(_context: Context, Json(item): Json<Item>) -> omnia_sdk::Result<Json<Item>> {
    let bucket = store::open(DEFAULT.to_owned()).await?;
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

Each `omnia:*` interface is a module of the bindings `wit-bindgen` generates, committed to the crate. A guest opens a resource on a binding the deployment declares and calls its methods. A module whose bindings a deployment names, such as `keyvalue` or `sql`, has a `DEFAULT` for the one a guest opens when it names no other.

- Absence is `Ok(None)`, not an error.
- A module's helpers sit beside the generated methods, such as `Bucket::get_json` and `Database::query_as`.
- `?` converts a binding's error into an `Error` under its code.

| Module | Entry point | Helpers |
| ------ | ----------- | ------- |
| `blobstore` | `store::open`, `admin::create_container` | Whole-object `Container::read`, `read_range`, and `write`. |
| `config` | `store::get`, `entries::list` | Typed `require` and `parse`, and `#[derive(Settings)]`. |
| `docstore` | `store::open` | Typed `Collection::get_as`, `insert`, and `upsert`, and `find` over a `Query`. |
| `http` | `fetch`, `send` | `fetch_json` for a JSON reply, and `serve` for an inbound request. |
| `identity` | `credentials::open` | `Identity::token`, an `AccessToken` with the time it has left. |
| `keyvalue` | `store::open` | JSON `Bucket::get_json` and `set_json`, and `compare_and_swap`. |
| `messaging` | `Client::connect`, `producer::send` | `Message::new` and `Message::json`, with a key and metadata. |
| `model` | `Request::complete` | `complete_with` for tool calls, and `Question` for a typed answer. |
| `plugins` | `Location::load` | A validated `Digest`, and the `Plugin` handle a load returns. |
| `sql` | `store::open`, `transactions::begin` | `query_as` through `FromRow`, and `exec` on a database or a transaction. |
| `vault` | `store::open` | None: a locker's generated methods. |
| `vcs` | `resolve`, `head`, `commit`, and the rest | One fn per operation over a deployment-local path. |
| `websocket` | `Client::connect`, `connections::send` | None: the generated sends and closes. |

`otel` holds the guest's telemetry library, and `#[omnia_sdk::otel::instrument]` opens a span around a fn. `mcp` serves the Model Context Protocol from a guest, behind the `http` feature.

## Transports

HTTP routes, messaging topics, and subcommands all register on a `Handlers`. A module registers its own on one, and the guest merges them with `Handlers::from([..])` or `merge` before `into_router` closes the whole into one `Router`. Two handlers for one path and method, one topic, or one subcommand are refused when they meet.

Each transport's `export!` binds the router fn to one WASI export. On native targets it expands to nothing, and a test or a native host serves the router itself.

### HTTP

An HTTP handler is a plain [axum](https://github.com/tokio-rs/axum) handler. It takes `Context` as an extractor beside axum's own, such as `Path`, `Query`, and `Json`. The `api::http` module documents the rest.

- `Handlers::route(path, method_router)` binds axum's `get`, `post`, `put`, `patch`, and `delete`, chained per path.
- `context.metadata` carries the `x-request-id`, `x-correlation-id`, `x-causation-id`, `x-tenant`, and `x-deadline` headers. A missing request id is minted.
- `http::export!` binds the `wasi:http` export, and serves each request inside the `handler` span `http::trace` opens.
- Omnia creates one component instance per request, so durable state belongs in a host interface, not a static.

```rust,ignore
use axum::Json;
use axum::extract::Path;
use axum::routing::get;
use omnia_sdk::api::{Context, Handlers};
use omnia_sdk::keyvalue::{DEFAULT, store};
use omnia_sdk::not_found;

async fn get_item(_context: Context, Path(id): Path<String>) -> omnia_sdk::Result<Json<Item>> {
    let bucket = store::open(DEFAULT.to_owned()).await?;
    let item = bucket.get_json(&id).await?;
    item.map(Json).ok_or_else(|| not_found!("no item {id}"))
}

pub fn handlers() -> Handlers {
    Handlers::new().route("/items/{id}", get(get_item))
}
```

### Messaging

A messaging handler is any `async fn(Context, I) -> Result<O, E>`, the context first as in an HTTP handler. A `Clone` closure of that shape is one too, so a closure can carry configuration. The `api::messaging` module documents the rest.

- `Handlers::topic(topic, handler)` decodes the payload as JSON.
- `Handlers::topic_with(topic, handler, decode)` takes a decoder over the whole `Delivery`, for any other wire format.
- A topic matches exactly, with no wildcards.
- `messaging::export!` binds the `omnia:messaging` handler.

```rust,ignore
use omnia_sdk::api::{Context, Handlers, Router, messaging};
use omnia_sdk::keyvalue::{DEFAULT, store};

async fn item_created(_context: Context, item: Item) -> omnia_sdk::Result<()> {
    let bucket = store::open(DEFAULT.to_owned()).await?;
    bucket.set_json(&item.id, &item, None).await
}

fn router() -> Router {
    Handlers::new().topic("items.created", item_created).into_router()
}

messaging::export!(router);
```

### Command

A subcommand is one more handler on `Handlers`, behind the `command` feature. Its input implements `clap::Args`, whose doc comments are the help text. Its output implements `Serialize` and `command::Render`. The `api::command` module documents the rest.

- `Handlers::command(name, handler)` registers a subcommand.
- `Handlers::command_with(command, handler, decode)` takes a `clap::Command` and a decoder over its `ArgMatches`, for any other grammar.
- A success goes to stdout at exit 0, in the format `--format text|json` selects.
- `--version` and `--help` answer the guest crate's version and description. `Router::version` and `Router::about` override them.
- `-v` and `-q` set the guest's log level, which the host reads from argv.
- `command::export!` binds the `wasi:cli/run` export, from a router fn or a zero-argument `async fn`. Omnia creates one component instance per invocation.

```rust,ignore
use clap::Args;
use omnia_sdk::api::{Context, Handlers, Router, command};
use omnia_sdk::keyvalue::{DEFAULT, store};

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
    store::open(DEFAULT.to_owned()).await?.set_json(&item.id, &item, None).await?;
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

An HTTP handler's error is anything `IntoResponse`, so a domain error with a wire shape of its own implements it. A topic or subcommand handler fails with any error type. The first `Error` in its source chain sets the variant and code, and any other error is a `ServerError` with the code `server_error`.

A binding's error converts into an `Error` whose code names the class of failure:

- `not-found`, `denied`, `invalid`, `unsupported`, or `other` on a `ServerError`.
- `unavailable` on a `BadGateway`, when the upstream did not answer or the call exceeded its time bound.

The [provider contract](https://github.com/augentic/omnia/blob/main/docs/reference/provider-contract.md#enforce-the-argument-rules) says which interfaces carry each code exactly. An error keeps its variant and code through any `anyhow` context around it.

Three interfaces fail with a typed error of their own. `?` converts it into an `Error` under the typed error's code, and a topic or subcommand reads it from the source chain as it reads an `Error`:

- `vcs` with `vcs::Error`
- `plugins` with `plugins::Error`
- `model` with `model::Error`

## Testing

A guest's host calls panic off `wasm32`, so a handler that reaches the host is tested as the compiled component:

- `omnia_test::host` runs the component through omnia's own runtime, over in-memory backends or scripted ones.
- `omnia_test::host::send` and `deliver` drive one request or one delivery through it in process.
- A handler that reaches no host interface also runs natively, through `Router::handle`, `Router::run`, or the `axum::Router` a `Router` converts into.

[Testing Omnia code](https://github.com/augentic/omnia/blob/main/docs/guides/testing-omnia-code.md) walks through each rung.

## Cargo features

- `http` *(default)*: the axum-backed `api::http` handlers, `Error: IntoResponse`, and the `mcp` server. Outbound `http::fetch` needs no feature.
- `command`: the clap-backed `api::command` program, with `Handlers::command`, `Router::run`, and shell completions. `command::export!` and the `Response` and `Failure` types need no feature.

Every interface is always built. A guest that serves no inbound HTTP can disable the default features to shrink its build. A command-only guest is `default-features = false, features = ["command"]`.

## License

MIT OR Apache-2.0
