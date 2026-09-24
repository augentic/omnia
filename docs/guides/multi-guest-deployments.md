# Multi-Guest Deployments

One Omnia runtime can run many guests at once. You can split inbound HTTP (or messaging, or WebSocket) across them, give them a host directory to work in, and let one guest call another. All of that is declared in a TOML manifest — you do not change guest or host code to add a second guest.

Start here once a single-guest runtime works ([Composing a Runtime](composing-a-runtime.md)). Passing one `.wasm` path to `run` is still the zero-config case. Use a manifest when you have more than one guest, or when you need routes or mounts.

This page walks through the manifest and the ideas behind it. The field-by-field schema is in [Configuration](../reference/configuration.md#deployment-manifest-omniatoml).

## The deployment manifest (`omnia.toml`)

Point the runtime at a manifest with `--manifest` (or the `OMNIA_MANIFEST` environment variable):

```bash
cargo run --example http-routing -- run --manifest examples/http-routing/omnia.toml
```

The `runtime!` macro can also compile a default deployment into the binary: a path via `manifest:`, or the manifest itself via the inline `guests` / `registries` / `mounts` keys, where each guest's component is embedded in the binary and carries its own `routes`. That default is used only when the command line supplies no source. See [Composing a Runtime](composing-a-runtime.md#default-manifest-manifest).

A manifest declares guests, mounts, routes, registries, and (eventually) transports. Every field is optional except at least one `[[guest]]`. Paths resolve relative to the manifest's own directory.

```toml
[[guest]]
name = "api"                            # the guest's name; the runtime treats it as a string
source.path = "./guests/api.wasm"       # .wasm or pre-compiled .bin
routes.http = ["/"]

[[guest]]
source.path = "./guests/admin.wasm"     # no `name`: the file's stem, `admin`, names it
routes.http = ["/admin"]
```

A guest's name is its identity: what routes address, what another guest dispatches to, and what a guest's `loader.load("api")` answers with. Leave it out when the file is called what callers dispatch to; give it when the file is not, or when two files share a stem. Names are unique within a deployment.

The full field reference lives in [Configuration](../reference/configuration.md#deployment-manifest-omniatoml).

## Programmatic manifests

Everything the TOML expresses can also be assembled in Rust. `omnia::Manifest` is the same schema as a value, with fluent setters for guests, mounts, registries, and routes. Pass it to the deployment builder (or the `runtime!`-generated `run(builder)`) instead of a file path:

```rust,ignore
use omnia::{DeploymentBuilder, GuestEntry, Manifest};

let manifest = Manifest::new()
    .guest(GuestEntry::new("api", "./guests/api.wasm").route_http("/"))
    .guest(GuestEntry::new("admin", "./guests/admin.wasm").route_http("/admin"));

host::run(DeploymentBuilder::new().manifest(manifest))?;
```

`Manifest::load(path)?` loads a TOML file into the same value, resolving relative paths against the file's directory. `Manifest::from_wasm(path)` is the one-guest shorthand (what you get from `run guest.wasm`). Relative paths in a programmatic manifest resolve against the process working directory.

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
source.path = "./guests/orders.wasm"     # exports the messaging handler; named `orders`
routes.messaging = ["orders.>"]          # orders.created, orders.cancelled, ...

[[guest]]
source.path = "./guests/billing.wasm"    # exports the messaging handler; named `billing`
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

## Guests calling guests

Two guests can talk without going out over the network. One guest **imports** an interface; another **exports** it; the host sits in the middle and copies the call across. Nothing in the manifest declares this — each component already says what it imports and exports:

```toml
[[guest]]
source.path = "./responder.wasm"        # exports example:link/echo
                                        # (named `responder` by its file)
[[guest]]
source.path = "./router.wasm"           # imports example:link/echo
```

The host decides what crosses by namespace: an interface under `wasi:` or `omnia:` is the host's own (WASI, and omnia's capability hosts), and every other interface a guest imports is relayed to whichever guest exports it. So `example:link/echo` — a package the guests' author owns — is wired between them without being named anywhere, and a guest package can never collide with the host's.

When `router` calls `echo`, the host starts a fresh `responder` instance, runs the export, and copies the result back. The two guests never share memory. The host does not understand what `echo` means — it only sees the interface name and the guest names.

A few constraints:

- Dispatch is deployment-wide. Any guest that imports an interface can call any guest that exports it; what a guest may reach is bounded by which guests the deployment runs, not by a list. An import no guest exports fails when the deployment boots, not at the first call. The exporter does not have to be present at startup: a guest registered later — or loaded through `omnia:plugins/loader` — can still serve the call. A runtime built without Omnia's `link` feature leaves such imports unsatisfied, so the deployment fails at boot.
- Calls can nest (A calls B calls C) up to `MAX_DISPATCH_DEPTH` (default 8), so accidental recursion cannot run forever.
- Today the call stays in-process. Declaring `unix`, `nats`, or `quic` under `[transport]` is rejected at load.

The [`guest-link`](../../examples/guest-link/) example is a complete caller/callee pair.

## How execution scales

All guests share one wasmtime engine (the compiler/runtime) and one linker (the table of host functions). Each guest is prepared once at startup. Every inbound request — and every guest-to-guest call — then gets a **new instance**, discarded afterwards. Guests never keep state in memory across calls; put durable state behind a capability such as key-value or SQL.

The pooling allocator (on by default) reuses the instance slots so this stays cheap. Tunables are in [Configuration](../reference/configuration.md#instance-pooling).
