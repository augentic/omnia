# Multi-Guest Deployments

One Omnia runtime can run many guests at once. You can split inbound HTTP (or messaging, or WebSocket) across them, give them a host directory to work in, and let one guest call another. All of that is declared in a TOML manifest — you do not change guest or host code to add a second guest.

Start here once a single-guest runtime works ([Composing a Runtime](composing-a-runtime.md)). Passing one `.wasm` path to `run` is still the zero-config case. Use a manifest when you have more than one guest, or when you need routes or mounts.

This page walks through the manifest and the ideas behind it. The field-by-field schema is in [Configuration](../reference/configuration.md#deployment-manifest-omniatoml).

## The deployment manifest (`omnia.toml`)

Point the runtime at a manifest with `--config` (or the `OMNIA_CONFIG` environment variable):

```bash
cargo run --example http-routing -- run --config examples/http-routing/omnia.toml
```

The `runtime!` macro can also compile a default deployment into the binary: a path via `config:`, or the manifest itself via the inline `link` / `plugin` / `guests` / `mounts` keys (each guest carries its own `routes`). That default is used only when the command line supplies no source. See [Composing a Runtime](composing-a-runtime.md#default-manifest-config).

A manifest declares guests, mounts, routes, and (eventually) transports. Every field is optional except at least one `[[guest]]`. Paths resolve relative to the manifest's own directory.

```toml
[[guest]]
id = "api"                              # a name you choose; the runtime treats it as a string
source.path = "./guests/api.wasm"       # .wasm or pre-compiled .bin
routes.http = ["/"]

[[guest]]
id = "admin"
source.path = "./guests/admin.wasm"
routes.http = ["/admin"]
```

The full field reference lives in [Configuration](../reference/configuration.md#deployment-manifest-omniatoml).

## Programmatic manifests

Everything the TOML expresses can also be assembled in Rust. `omnia::Manifest` is the same schema as a value, with fluent setters for guests, mounts, link interfaces, and routes. Pass it to the deployment builder (or the `runtime!`-generated `run(builder)`) instead of a file path:

```rust,ignore
use omnia::{DeploymentBuilder, GuestEntry, Manifest};

let manifest = Manifest::new()
    .link(["omnia:link/audit"])
    .guest(GuestEntry::new("api", "./guests/api.wasm").route_http("/"))
    .guest(GuestEntry::new("admin", "./guests/admin.wasm").route_http("/admin"));

host::run(DeploymentBuilder::new().manifest(manifest))?;
```

`Manifest::from_config(path)?` loads a TOML file into the same value, resolving relative paths against the file's directory. `Manifest::from_wasm(path)` is the one-guest shorthand (what you get from `run guest.wasm`). Relative paths in a programmatic manifest resolve against the process working directory.

The [`guest-link-dynamic`](../../examples/guest-link/dynamic.rs) example builds a host this way.

## Routing inbound traffic

This is the analogue of putting two services behind one reverse proxy. Each guest declares the routes that should reach it, one optional list per trigger. The runtime builds those into a route table per trigger at load:

- **`routes.http`** — path prefixes, longest prefix wins. One HTTP server fronts all guests.
- **`routes.messaging`** — topics matched by NATS-style pattern (`.`-separated tokens, `*` matches one token, `>` matches the rest).
- **`routes.websocket`** — same pattern syntax, for WebSocket routes.

If a trigger has no routes and exactly one guest exports its handler, that guest is the catch-all — so a single-guest deployment needs no routes at all.

The [`http-routing`](../../examples/http-routing/) example runs two HTTP guests behind `/a` and `/b` prefixes.

A messaging deployment works the same way. The host backend subscribes to topics (broker configuration such as `KAFKA_TOPICS`/`NATS_TOPICS`, or everything for the in-memory default). Each guest's route list then picks which of those delivered messages it handles:

```toml
[[guest]]
id = "orders"
source.path = "./guests/orders.wasm"     # exports the messaging handler
routes.messaging = ["orders.>"]          # orders.created, orders.cancelled, ...

[[guest]]
id = "billing"
source.path = "./guests/billing.wasm"    # exports the messaging handler
routes.messaging = ["invoices.*"]        # exactly one token after `invoices.`
```

Each matched message starts a fresh instance of the routed guest, exactly like an HTTP request. Inside the guest, topic-to-handler matching stays exact — see [Messaging](messaging.md#handling-incoming-messages).

## Mounts: giving guests a workspace

Guests cannot see the host filesystem unless you say so. A **mount** takes a host directory and makes it visible inside every guest, under a name the guest can look up:

```toml
[[mount]]
name = "."          # what the guest sees in preopens.get-directories()
path = "../.."      # host path, relative to the manifest
writable = true     # omit for read-only (the default)
```

The equivalent on the command line (repeatable, layered over the manifest, last-wins per name):

```bash
cargo run --example cli -- run guest.wasm --mount path=workspace,name=.,writable
```

Guests discover mounts through `wasi:filesystem/preopens`:

```rust
let directories = preopens::get_directories();
let workspace = directories.iter().find_map(|(dir, name)| (name == ".").then_some(dir));
```

The [`model`](../../examples/model/) example lends a mounted workspace to a model backend this way.

## Guest-to-guest linking

Two guests can talk without going out over the network. One guest **imports** an interface; another **exports** it; the host sits in the middle and copies the call across. Name the interface in `[link] interfaces` so the host knows to wire it:

```toml
[link]
interfaces = ["omnia:link/echo"]

[[guest]]
id = "responder"
source.path = "./responder.wasm"        # exports omnia:link/echo

[[guest]]
id = "router"
source.path = "./router.wasm"           # imports omnia:link/echo
```

When `router` calls `echo`, the host starts a fresh `responder` instance, runs the export, and copies the result back. The two guests never share memory. The host does not understand what `echo` means — it only sees the interface name and the guest ids.

A few constraints:

- The allow-list is deployment-wide. Any guest that imports a listed interface can call any guest that exports it. `--link <interface>` on the command line is added to the manifest's list. The exporter does not have to be present at startup; a guest registered later can still serve the call. A runtime built without Omnia's `link` feature refuses a non-empty list.
- Calls can nest (A calls B calls C) up to `MAX_DISPATCH_DEPTH` (default 8), so accidental recursion cannot run forever.
- Today the call stays in-process. Declaring `unix`, `nats`, or `quic` under `[transport]` is rejected at load.

The [`guest-link`](../../examples/guest-link/) example is a complete caller/callee pair.

## How execution scales

All guests share one wasmtime engine (the compiler/runtime) and one linker (the table of host functions). Each guest is prepared once at startup. Every inbound request — and every guest-to-guest call — then gets a **new instance**, discarded afterwards. Guests never keep state in memory across calls; put durable state behind a capability such as key-value or SQL.

The pooling allocator (on by default) reuses the instance slots so this stays cheap. Tunables are in [Configuration](../reference/configuration.md#instance-pooling).
