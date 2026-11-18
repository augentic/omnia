# omnia-sdk

`omnia-sdk` is the SDK for writing an Omnia guest, a WASI component that serves HTTP requests, messaging deliveries, and command lines. Its handlers are generic over a provider of host capabilities, so one handler runs over the WASI host in a guest and over test doubles natively.

## Quick start

A handler is an `async fn` that takes a `Context<P>`, where `P` is the provider of the capabilities the handler reaches. Register it on `Handlers`, close the handlers over a provider with `with_state(provider)`, and bind the router fn with `http::export!`.

```rust,ignore
use axum::Json;
use axum::routing::post;
use omnia_sdk::api::{Context, Handlers, Router, http};
use omnia_sdk::{StateStore, StateStoreExt};
use serde::{Deserialize, Serialize};

#[derive(Deserialize, Serialize)]
struct Item {
    id: String,
    name: String,
}

async fn create_item<P: StateStore>(
    context: Context<P>, Json(item): Json<Item>,
) -> omnia_sdk::Result<Json<Item>> {
    context.state().set_json(&item.id, &item, None).await?;
    Ok(Json(item))
}

pub fn router<P: StateStore + 'static>(provider: P) -> Router<P> {
    Handlers::new().route("/items", post(create_item)).with_state(provider)
}

http::export!(router);
```

[Writing guests](https://github.com/augentic/omnia/blob/main/docs/guides/writing-guests.md) walks through a whole guest.

## Transports

HTTP routes, messaging topics, and subcommands all register on a `Handlers<P>`. A module registers its own on one, and the guest merges them with `Handlers::from([..])` or `merge` before `with_state` closes the whole into one `Router<P>`. Two handlers for one path and method, one topic, or one subcommand are refused when they meet.

A module's `handlers<P>()` bounds `P` on a `Requires` trait of its own. It names the capabilities the module's handlers reach, plus `Send + Sync + 'static`, with a blanket impl for every provider that has them.

Each transport's `export!` binds the router fn to one WASI export. It passes `WasiProvider`, or the provider given as a second argument. On native targets it expands to nothing, and a test or a native host serves the router itself.

### HTTP

An HTTP handler is a plain [axum](https://github.com/tokio-rs/axum) handler. It takes `Context<P>` as an extractor beside axum's own, such as `Path`, `Query`, and `Json`. The `api::http` module documents the rest.

- `Handlers::route(path, method_router)` binds axum's `get`, `post`, `put`, `patch`, and `delete`, chained per path.
- A handler bounds `P` on the capability traits it reaches and nothing more. The router supplies the `'static` axum needs.
- `context.metadata` carries the `x-request-id`, `x-correlation-id`, `x-causation-id`, `x-tenant`, and `x-deadline` headers. A missing request id is minted.
- `http::export!` binds the `wasi:http` export, and serves each request inside the `handler` span `http::trace` opens.
- Omnia creates one component instance per request, so durable state belongs in a capability, not a static.

```rust,ignore
use axum::Json;
use axum::extract::Path;
use axum::routing::get;
use omnia_sdk::api::{Context, Handlers};
use omnia_sdk::{StateStore, StateStoreExt, not_found};

async fn get_item<P: StateStore>(
    context: Context<P>, Path(id): Path<String>,
) -> omnia_sdk::Result<Json<Item>> {
    let item = context.state().get_json(&id).await?;
    item.map(Json).ok_or_else(|| not_found!("no item {id}"))
}

async fn delete_item<P: StateStore>(
    context: Context<P>, Path(id): Path<String>,
) -> omnia_sdk::Result<()> {
    context.state().delete(&id).await
}

pub fn handlers<P: StateStore + 'static>() -> Handlers<P> {
    Handlers::new().route("/items/{id}", get(get_item).delete(delete_item))
}
```

### Messaging

A messaging handler is any `async fn(Context<P>, I) -> Result<O, E>`, the context first as in an HTTP handler. A `Clone` closure of that shape is one too, so a closure can carry configuration. The `api::messaging` module documents the rest.

- `Handlers::topic(topic, handler)` decodes the payload as JSON.
- `Handlers::topic_with(topic, handler, decode)` takes a decoder over the whole `Delivery`, for any other wire format.
- A topic matches exactly, with no wildcards.
- `messaging::export!` binds the `omnia:messaging` handler.

```rust,ignore
use omnia_sdk::api::{Context, Handlers, Router, messaging};
use omnia_sdk::{StateStore, StateStoreExt};

async fn item_created<P: StateStore>(context: Context<P>, item: Item) -> omnia_sdk::Result<()> {
    context.state().set_json(&item.id, &item, None).await
}

pub fn router<P: StateStore + 'static>(provider: P) -> Router<P> {
    Handlers::new().topic("items.created", item_created).with_state(provider)
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
- `command::export!` binds the `wasi:cli/run` export. Omnia creates one component instance per invocation.

```rust,ignore
use clap::Args;
use omnia_sdk::api::{Context, Handlers, Router, command};
use omnia_sdk::{StateStore, StateStoreExt};

/// Create an item.
#[derive(Args)]
struct Create {
    /// The item's id.
    id: String,

    /// The item's name.
    name: String,
}

async fn create<P: StateStore>(context: Context<P>, input: Create) -> omnia_sdk::Result<String> {
    let item = Item { id: input.id, name: input.name };
    context.state().set_json(&item.id, &item, None).await?;
    Ok(format!("created {}", item.id))
}

pub fn router<P: StateStore + 'static>(provider: P) -> Router<P> {
    Handlers::new().command("create", create).with_state(provider)
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

A capability call fails with an `Error` whose code names the class of failure:

- `not-found`, `denied`, `invalid`, `unsupported`, or `other` on a `ServerError`.
- `unavailable` on a `BadGateway`, when the upstream did not answer or the call exceeded its time bound.

The [provider contract](https://github.com/augentic/omnia/blob/main/docs/reference/provider-contract.md#enforce-the-argument-rules) says which interfaces carry each code exactly. A capability error keeps its variant and code through any `anyhow` context around it.

Three capabilities fail with a typed error of their own. `?` converts it into an `Error` under the typed error's code, and a topic or subcommand reads it from the source chain as it reads an `Error`:

- `Vcs` with `vcs::Error`
- `Plugins` with `plugins::Error`
- `Model` with `model::Error`

## Capabilities

A capability is a trait over one host service. `WasiProvider` implements every one on `wasm32` through the WASI host. A handler bounds its `P` on the traits it uses and reaches each through its `Context` accessor, so it never names the provider.

- Absence is `Ok(None)`, not an error.
- Bytes are `Bytes`.
- `Identity`, `SecretStore`, `SqlDb`, and `StateStore` address the binding `default`. `named(name)` answers a handle on another binding the deployment declares.
- A `<Capability>Ext` trait holds the typed codecs over a capability's primitives, such as `StateStoreExt::get_json`.

| Trait | Accessor | Purpose |
| ----- | -------- | ------- |
| `BlobStore` | `blobs()` | Objects in containers, with ranged reads and paged listings. |
| `Config` | `config()` | Configuration values, read typed through `ConfigExt` or as a `#[derive(Settings)]` struct. |
| `DocumentDb` | `docs()` | JSON documents by id, with filtered, sorted, and paged queries. |
| `HttpClient` | `http()` | Outbound HTTP requests, with JSON replies through `HttpClientExt::fetch_json`. |
| `Identity` | `identity()` | Access tokens for the identities the deployment declares. |
| `Model` | `model()` | Model completions, with tool calls a guest closure answers. |
| `Plugins` | `plugins()` | Loads of other guests by declared name, mounted path, or registry package. |
| `Publisher` | `publisher()` | Messages published to a topic, with an optional key that orders them. |
| `SecretStore` | `secrets()` | Secrets in one locker, to read, write, delete, check, and list. |
| `SqlDb` | `sql()` | Parameterized SQL and transactions, with rows decoded through `SqlDbExt::query_as`. |
| `StateStore` | `state()` | Key-value state with TTLs, compare-and-swap, and atomic increments. |
| `Vcs` | `vcs()` | Version-control operations over lent repository and working-copy mounts. |
| `WebSockets` | `websockets()` | Data sent to every connected WebSocket, or to the sockets named. |

`#[derive(omnia_sdk::Provider)]` builds a provider of your own from fields that serve capabilities:

- A field annotated `#[provider(Config, StateStore)]` serves the capabilities it names.
- A path names a capability declared outside `omnia_sdk`, as `#[provider(crate::geo::Geocoder)]`. A crate declares one with `#[omnia_sdk::capability]`.
- A capability's associated types, such as `StateStore::Named`, are the field's own.
- `Arc<T>`, `&T`, and `Box<T>` serve every capability `T` serves.
- A `pub` provider needs `pub` field types.

## Testing

A handler test runs natively, over the doubles in `omnia_test::guest` and with no WASI host:

- `Provider` holds one seeded double per capability, behind every capability trait.
- `send(&router, request)` drives one HTTP request through a router and collects the reply.
- `deliver(&router, topic, payload)` drives one messaging delivery.
- `Router::run(argv)` runs one command line and answers its output and exit code.
- `Context::new(provider, metadata)` builds a context for a fn that takes one, without a router.

[Testing Omnia code](https://github.com/augentic/omnia/blob/main/docs/guides/testing-omnia-code.md) covers the next rung, which runs the compiled component through omnia's own runtime.

## Cargo features

- `http` *(default)*: the axum-backed `api::http` handlers, `Error: IntoResponse`, and the `mcp` server. The outbound `HttpClient` needs no feature.
- `command`: the clap-backed `api::command` program, with `Handlers::command`, `Router::run`, and shell completions. `command::export!` and the `Response` and `Failure` types need no feature.

Every capability is always built. A guest that serves no inbound HTTP can disable the default features to shrink its build. A command-only guest is `default-features = false, features = ["command"]`.

## License

MIT OR Apache-2.0
