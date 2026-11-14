# Provider Contract

What a provider owes the application beyond the capability trait signatures, and how a host outside omnia serves a guest's router. The traits are introduced in [Writing Guests](../guides/writing-guests.md#using-wasi-capabilities), and the rows a hand-written provider runs are covered in [Testing Omnia-Based Code](../guides/testing-omnia-code.md#handler-rung-omnia_testguestprovider).

- [What a provider is](#what-a-provider-is)
- [Serving the router natively](#serving-the-router-natively)
- [Obligations](#obligations)
- [The application's half](#the-applications-half)

## What a provider is

A provider is the one value a guest's handlers run against. It implements the capability traits (`StateStore`, `Config`, `SqlDb`, ...) their bounds name, and the `Client` a `Router` closes over owns it. A handler never names it. Three kinds serve the same handlers:

- `omnia_sdk::WasiProvider` serves every capability through the WASI host, inside a guest component on `wasm32`.
- `omnia_test::guest::Provider` serves them natively with in-memory doubles, for handler tests.
- A native provider, written outside omnia, serves them over real services in a native host that links the guest crate as an `rlib`.

Omnia ships no native provider. The traits are the application's contract with any provider, and this page states what a provider and the host serving it owe. Omnia is the reference: each obligation says how it meets that obligation for `WasiProvider`.

## Serving the router natively

A guest's `Router<P>` serves every transport through public API. On native targets each `export!` expands to nothing, and the host takes over what the export does on `wasm32`: it owns the provider, sets each invocation's metadata, and drives the router.

### HTTP

The router is a `tower::Service`, and `axum::Router::from(router)` is the plain axum router behind it. The host layers `api::http::trace`, as `http::export!` does, and sets `x-deadline` on every request ahead of it:

```rust,ignore
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use axum::extract::Request;
use axum::http::HeaderValue;
use axum::middleware::{from_fn, map_request};
use omnia_sdk::api::http::trace;

const BUDGET: Duration = Duration::from_secs(30);

async fn stamp_deadline(mut request: Request) -> Request {
    let deadline = (SystemTime::now() + BUDGET).duration_since(UNIX_EPOCH).unwrap_or_default();
    let millis = u64::try_from(deadline.as_millis()).unwrap_or(u64::MAX);
    request.headers_mut().insert("x-deadline", HeaderValue::from(millis));
    request
}

let app = axum::Router::from(http_router(provider))
    .layer(from_fn(trace))
    .layer(map_request(stamp_deadline));
```

The deadline layer is added last, so it runs first and `trace` reads the stamped header. The host still enforces the budget itself, with a timeout around each request.

### Messaging

The host subscribes to each of `Router::topics` and hands every delivery to `Router::handle`. Before it does, it sets the delivery's `deadline` metadata entry, replacing any entry of that name the broker delivered, whatever its case. It then acts on the `DeliveryError` a failure returns:

- It drops an `Undecodable` delivery, which no redelivery decodes.
- It redelivers a `Failed` delivery of kind `ErrorKind::Unavailable`, which may succeed later.

What becomes of any other failure is the host's own policy.

### Commands

`Router::run(argv)` reads `Metadata::from_env` under the program's prefix, and `Router::run_with(argv, &metadata)` takes the metadata explicitly. The host writes the returned `Response`'s `stdout` and `stderr`, then exits with its `exit`. A command runs uncapped, so it carries a deadline only when the host sets `<PREFIX>_DEADLINE` or passes one through `run_with`.

### Deadline carriers

Every carrier holds the deadline as decimal milliseconds since the Unix epoch:

| Transport | Carrier |
| --------- | ------- |
| HTTP | The `x-deadline` header |
| Messaging | The `deadline` metadata entry |
| Command | The `<PREFIX>_DEADLINE` environment variable |

`Metadata::from_lookup` parses the carrier into `Metadata::deadline`, and a value that does not parse reads as none. The host's stamp replaces any value the caller sent, because the deadline is the host's budget for the invocation.

## Obligations

What any provider owes the application beyond the trait signatures, each with how Omnia meets it.

### Pass the shared rows

Each behavioural invariant of a capability trait is one generic `async fn` in `omnia_test::contract` (behind the `contract` feature), which panics when the provider breaks it. A provider conforms when the rows of every trait it implements pass over it. Omnia runs the rows natively over its stateful doubles, and through `test-programs` guests against its hosts and backends.

### Enforce the argument rules

The same arguments fail the same way on every provider:

- A zero TTL fails with `ErrorKind::Invalid`, and a TTL rounds up to the store's resolution.
- A handle that names no binding addresses `default`, and a name the deployment does not declare fails with `ErrorKind::NotFound`. A secret locker is the exception, and opens under any name.
- A document query whose filter `Filter::check` refuses fails with `ErrorKind::Invalid`.
- A call on an object in a blob container that was never created fails with `ErrorKind::NotFound`.
- A request the provider does not serve fails with `ErrorKind::Unsupported`, and is never partly served. The [portable SQL dialect](sql.md#the-portable-dialect) and the [document query contract](docstore.md#query-contract) list what every provider serves.

Omnia enforces each rule in the layer this table names, so a backend owes only the rules given to it:

| Rule | Enforced by |
| ---- | ----------- |
| A zero TTL is `Invalid` | The `omnia:keyvalue` host, for every backend. |
| A TTL rounds up | The SDK, to milliseconds, then each backend, to its store's resolution. |
| A handle that names no binding is `default` | The SDK. |
| An undeclared name is `NotFound` | Each backend, when it opens a bucket, a database, or an identity. |
| A filter past its limits is `Invalid` | The SDK, before the query reaches the host. The host traps a guest that passes the raw `filter` constructors past the same limits. |
| An uncreated container is `NotFound` | Each blob backend. |
| An unserved request is `Unsupported` | Each backend. |

The kind a backend picks reaches the guest exactly over `omnia:keyvalue`, `omnia:sql`, and `omnia:docstore`, which carry one error case per `ErrorKind`. `omnia:identity` carries `NotFound` and `Other`. Elsewhere a kind is best effort:

- `wasi:blobstore` errors are strings, so the SDK reads a missing container or object from the message.
- The other `wasi:*` interfaces have fewer error cases than `ErrorKind` has kinds, and the SDK maps each case to one kind.

### Bound every call

No capability call waits for ever, and a call that exceeds its bound fails with `ErrorKind::Unavailable`. Handlers set no timeouts of their own, so the transport also sets `Metadata::deadline` to the handler's budget. Omnia caps each HTTP and messaging invocation at `GUEST_TIMEOUT_MS` (30 seconds by default; see [Guest limits](configuration.md#guest-limits)) and stamps that expiry as the deadline. A command runs uncapped and carries none.

### Serve concurrent handlers from one provider

The provider is `Send + Sync + 'static`, and `Client::new` shares the one value behind an `Arc` with every handler the router runs. Handlers keep no global state. Omnia instantiates a fresh guest for each request, so a native host is the first to share one provider across concurrent handlers.

### Mint request ids from a CSPRNG

A transport that mints a request id draws it from a CSPRNG. `Metadata::from_lookup` mints a missing id from the platform's CSPRNG on every target: `wasi:random` in a guest, and the operating system natively.

### Apply the tracing layer

A host that serves the router itself layers `api::http::trace`, so each request runs inside the `handler` span carrying its ids, method, and route. `http::export!` applies it inside a guest. A delivery and a subcommand open the same span through the router's own dispatch.

## The application's half

The guest `clippy.toml` that omnia-exemplar's [`templates/guest`](https://github.com/augentic/omnia-exemplar/blob/main/templates/guest/core/clippy.toml) ships bans the process, clock, file system, network, and environment APIs a handler could reach around its provider. Code that passes those bans reaches the outside world only through the traits, so any provider that keeps this contract can serve it.
