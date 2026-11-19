# Provider Contract

What a provider owes the application beyond the WIT signatures, and what each transport sets on an invocation. A guest reaches a provider only through the bindings in `omnia_sdk` ([Writing Guests](../guides/writing-guests.md#using-host-interfaces)), and a backend proves it keeps this contract with the rows in [Testing Omnia-Based Code](../guides/testing-omnia-code.md#contract-rows).

- [What a provider is](#what-a-provider-is)
- [What each transport sets](#what-each-transport-sets)
- [Obligations](#obligations)
- [The application's half](#the-applications-half)

## What a provider is

A provider serves one `omnia:*` interface to a guest: the WASI host for the interface, such as `WasiKeyValue`, and the backend it runs over, such as `KeyValueDefault` or a production client from `omnia-backends`. A guest never names one; the deployment decides what backs each interface. Omnia's in-memory defaults are the reference, and each obligation says how omnia meets it.

## What each transport sets

A trigger host sets each invocation's metadata before the guest's router reads it as `Context::metadata`.

### HTTP

The `wasi:http` host stamps `x-deadline` on every request and enforces the budget itself, with a timeout around each request. In the guest, `http::export!` serves the request inside the `handler` span `http::trace` opens, which reads the stamped header.

### Messaging

The messaging host sets each delivery's `deadline`, which the router passes on as `Metadata::deadline`. A failure settles the delivery, with the `omnia_sdk::Error`'s text as the reason:

- A `BadRequest`, such as the code `undecodable`, rejects it, since no redelivery handles it.
- Any other failure releases it, since it may succeed later.

What a backend does with a rejected or released delivery is its own policy.

### Commands

`Router::run(argv)` reads `Metadata::from_env` under the program's prefix, and `Router::run_with(argv, &metadata)` takes the metadata explicitly. `command::export!` writes the returned `Response`'s `stdout` and `stderr`, then exits with its `exit`. A command runs uncapped, so it carries a deadline only when its environment sets `<PREFIX>_DEADLINE`.

### Deadline carriers

Every carrier holds the deadline as milliseconds since the Unix epoch:

| Transport | Carrier |
| --------- | ------- |
| HTTP | The `x-deadline` header, as decimal text |
| Messaging | The delivery's `deadline` |
| Command | The `<PREFIX>_DEADLINE` environment variable, as decimal text |

`Metadata::from_lookup` parses a text carrier into `Metadata::deadline`, and a value that does not parse reads as none. The host's stamp replaces any value the caller sent, because the deadline is the host's budget for the invocation.

## Obligations

What any provider owes the application beyond the WIT signatures, each with how Omnia meets it.

### Pass the shared rows

Each behavioural invariant of an interface is one `async fn` in `omnia_test::contract` (behind the `contract` feature), which panics when the backend breaks it. A guest runs the rows over the bindings, against a host and its backend. A backend conforms when the rows of every interface it serves pass over it. Omnia runs them through `test-programs` guests over its in-memory defaults.

### Enforce the argument rules

The same arguments fail the same way on every provider. Each failure is the shared `types::Error`, whose case names its class. `?` converts it into an `omnia_sdk::Error` under that code, a `ServerError` for every code but `unavailable`, which is a `BadGateway`:

- A zero TTL fails with the code `invalid`, and a TTL rounds up to the store's resolution.
- A name the deployment does not declare fails with the code `not-found`.
- A document query whose filter `Filter::check` refuses fails with the code `invalid`.
- A call on an object in a blob container that was never created fails with the code `not-found`.
- A request the provider does not serve fails with the code `unsupported`, and is never partly served. The [portable SQL dialect](sql.md#the-portable-dialect) and the [document query contract](docstore.md#query-contract) list what every provider serves.

Omnia enforces each rule in the layer this table names, so a backend owes only the rules given to it:

| Rule | Enforced by |
| ---- | ----------- |
| A zero TTL is `invalid` | The `omnia:keyvalue` host, for every backend. |
| A TTL rounds up | The SDK, to milliseconds, then each backend, to its store's resolution. |
| An undeclared name is `not-found` | Each backend, when it opens a bucket, a database, an identity, or a locker. |
| A filter past its limits is `invalid` | The SDK, before the query reaches the host. The host traps a guest that passes the raw `filter` constructors past the same limits. |
| An uncreated container is `not-found` | Each blob backend. |
| An unserved request is `unsupported` | Each backend. |

The code a backend picks reaches the guest exactly over `omnia:keyvalue`, `omnia:blobstore`, `omnia:sql`, `omnia:docstore`, `omnia:identity`, and `omnia:vault`, which carry one error case per code. Elsewhere a code is best effort: the other `wasi:*` interfaces have fewer error cases than there are codes, and the SDK maps each case to one code.

### Bound every call

No host call waits for ever, and a call that exceeds its bound fails with the code `unavailable`. Handlers set no timeouts of their own, so the transport also sets `Metadata::deadline` to the handler's budget. Omnia caps each HTTP and messaging invocation at `GUEST_TIMEOUT_MS` (30 seconds by default; see [Guest limits](configuration.md#guest-limits)) and stamps that expiry as the deadline. A command runs uncapped and carries none.

### Mint request ids from a CSPRNG

A transport that mints a request id draws it from a CSPRNG. `Metadata::from_lookup` mints a missing id from the platform's CSPRNG on every target: `wasi:random` in a guest, and the operating system natively.

### Apply the tracing layer

Each request runs inside the `handler` span `http::trace` opens, carrying its ids, method, and route. `http::export!` and `http::serve` apply it inside a guest, and a host that serves a `Router` natively layers it itself. A delivery and a subcommand open the same span through the router's own dispatch.

## The application's half

The guest `clippy.toml` that omnia-exemplar's [`templates/guest`](https://github.com/augentic/omnia-exemplar/blob/main/templates/guest/core/clippy.toml) ships bans the process, clock, file system, network, and environment APIs a handler could reach around the host interfaces. Code that passes those bans reaches the outside world only through the bindings, so any provider that keeps this contract can serve it.
