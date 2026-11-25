# Multi-Guest Deployments

One Omnia runtime can run many guests at once. You can split inbound HTTP (or messaging, or WebSocket) across them, give them a host directory to work in, let one guest call another, and narrow what each one reaches. All of that is declared in a TOML manifest — you do not change guest or host code to add a second guest.

Start here once a single-guest runtime works ([Composing a Runtime](composing-a-runtime.md)). Passing one `.wasm` path to `run` is still the zero-config case. Use a manifest when you have more than one guest, or when you need routes or mounts.

This page walks through the manifest and the ideas behind it. The field-by-field schema is in [Configuration](../reference/configuration.md#deployment-manifest-omniatoml).

## The deployment manifest (`omnia.toml`)

Point the runtime at a manifest with `--manifest` (or the `OMNIA_MANIFEST` environment variable):

```bash
cargo run --example http-routing -- run --manifest examples/http-routing/omnia.toml
```

The `runtime!` macro can also compile a default deployment into the binary: a path via `manifest:`, or the manifest itself via the inline `guests` / `mounts` / `plugins` keys, where each guest's component is embedded in the binary and carries its own `routes`. That default is used only when the command line supplies no source. See [Composing a Runtime](composing-a-runtime.md#default-manifest-manifest).

A manifest declares guests, mounts, routes, each guest's grants, the package store and registries, and (eventually) transports. Every field is optional except at least one `[[guest]]`. Paths resolve relative to the manifest's own directory.

```toml
[[guest]]
name = "api"                            # the guest's name; the runtime treats it as a string
source.path = "./guests/api.wasm"       # raw .wasm (a pre-compiled .bin needs a `digest`)
routes.http = ["/"]

[[guest]]
source.path = "./guests/admin.wasm"     # no `name`: the file's stem, `admin`, names it
routes.http = ["/admin"]
```

A guest's name is its identity: what routes address, what another guest dispatches to, and what a guest's `loader.load("api")` answers with. Leave it out when the file is called what callers dispatch to; give it when the file is not, or when two files share a stem. Names are unique within a deployment.

A manifest file's guest is not compiled into the runtime, so it loads at its first use — the first request its routes match, the command run that drives it, the first call another guest makes to it — rather than at startup. Startup validates the manifest; the file is read, pinned, and compiled when the guest is first named, and a failure there (a missing file, a stale `digest`) fails that use. A guest nothing names — no routes, no `command = true`, no caller — never runs, since only the guests compiled into the runtime can be a trigger's catch-all.

The full field reference lives in [Configuration](../reference/configuration.md#deployment-manifest-omniatoml).

## Programmatic manifests

Everything the TOML expresses can also be assembled in Rust. `omnia::Manifest` is the same schema as a value, with fluent setters for guests, mounts, the store, registries, routes, and grants (`GuestEntry::grants`, and `Manifest::plugin_grants` for a guest loaded at run time). Pass it to the deployment builder (or the `runtime!`-generated `run(builder)`) instead of a file path:

```rust,ignore
use omnia::{DeploymentBuilder, GuestEntry, Manifest};

let manifest = Manifest::new()
    .guest(GuestEntry::new("api", "./guests/api.wasm").route_http("/"))
    .guest(GuestEntry::new("admin", "./guests/admin.wasm").route_http("/admin"));

host::run(DeploymentBuilder::new().manifest(manifest))?;
```

`Manifest::load(path)?` loads a TOML file into the same value, resolving relative paths against the file's directory. `Manifest::from_wasm(path)?` is the one-guest shorthand (what you get from `run guest.wasm`): it reads the component now, so that guest loads at boot like an embedded one. A `GuestEntry::new(name, path)` is a first-use guest exactly as a `[[guest]]` file entry is; `GuestEntry::new(name, bytes)` loads at boot. Relative paths in a programmatic manifest resolve against the process working directory.

The [`guest-link-dynamic`](../../examples/guest-link/dynamic.rs) example builds a host this way.

## Routing inbound traffic

This is the analogue of putting two services behind one reverse proxy. Each guest declares the routes that should reach it, one optional list per trigger. The runtime builds those into a route table per trigger at load:

- **`routes.http`** — path prefixes, longest prefix wins. One HTTP server fronts all guests.
- **`routes.messaging`** — topics matched by NATS-style pattern (`.`-separated tokens, `*` matches one token, `>` matches the rest).
- **`routes.websocket`** — same pattern syntax, for WebSocket routes.

If a trigger has no routes and exactly one guest compiled into the runtime (an embedded `path:`, the `run <component>` file) exports its handler, that guest is the catch-all — so a single-guest deployment run from the command line needs no routes at all. A manifest file's guest loads at first use and is never the catch-all: give it routes, or the `command = true` mark.

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

Each matched message starts a fresh instance of the routed guest, exactly like an HTTP request; the first message to match a guest's routes is what loads it. Inside the guest, topic-to-handler matching stays exact — see [Messaging](messaging.md#handling-incoming-messages).

## Mounts: giving guests a workspace

Guests cannot see the host filesystem unless you say so. A **mount** takes a host directory and makes it visible inside every guest, under a name the guest can look up. A guest with [grants](#grants-what-each-guest-reaches) sees only the mounts they name.

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

A read-only mount is also where the guest loader (`omnia:plugins/loader`, behind the `loader` feature) reads a component a guest whose grants reach the mount names by path; a writable mount never is, and a writable mount that shares or nests a read-only mount's directory is refused at startup. Mount code read-only and give state a mount of its own — see the [security model](../security-model.md#guest-requested-plugin-loading-omniapluginsloader).

## Guests calling guests

Two guests can talk without going out over the network. One guest **imports** an interface; another **exports** it; the host sits in the middle and copies the call across. Nothing in the manifest declares this — each component already says what it imports and exports:

```toml
[[guest]]
source.path = "./responder.wasm"        # exports example:link/echo
                                        # (named `responder` by its file)
[[guest]]
source.path = "./router.wasm"           # imports example:link/echo
command = true                          # the run drives `router`, which loads it
```

The host decides what crosses by namespace: an interface under `wasi:` or `omnia:` is the host's own (WASI, and omnia's capability hosts), and every other interface a guest imports is relayed to whichever guest exports it. So `example:link/echo` — a package the guests' author owns — is wired between them without being named anywhere, and a guest package can never collide with the host's.

When `router` calls `echo`, the host starts a fresh `responder` instance, runs the export, and copies the result back. The two guests never share memory. The host does not understand what `echo` means — it only sees the interface name and the guest names.

A few constraints:

- Dispatch is deployment-wide. A guest without grants that imports an interface can call any guest that exports it, so what it may reach is bounded by which guests the deployment runs. A guest with [grants](#grants-what-each-guest-reaches) imports the interface only when its `interfaces` name the package, and calls only the guests its `guests` name. An import no guest exports is relayed too, so the deployment boots and the call fails. The exporter does not have to be loaded at startup: a manifest file's guest loads when the first call names it (so above, `router`'s first `echo` is what loads `responder`), and a guest registered later — or loaded through `omnia:plugins/loader` — can serve the call too. A runtime built without Omnia's `link` feature leaves such imports unsatisfied, so the deployment fails at boot.
- Calls can nest (A calls B calls C) up to `MAX_DISPATCH_DEPTH` (default 8), so accidental recursion cannot run forever.
- Each call takes an instance from the pool but no slot of the [concurrency gate](../reference/configuration.md#concurrency-gate), so set `MAX_CONCURRENT_INVOCATIONS` below `POOL_MAX_INSTANCES` by the margin [Pool sizing](performance-tuning.md#2-pool-sizing) works out.
- Today the call stays in-process. Declaring `unix`, `nats`, or `quic` under `[transport]` is rejected at load.

The [`guest-link`](../../examples/guest-link/) example is a complete caller/callee pair.

## Grants: what each guest reaches

By default a guest reaches the whole deployment: every variable `[env] inherit` passes, every mount, and every capability the runtime links. That suits guests you wrote. When one guest is trusted less than the rest, such as a third-party adapter, give it a grants table, and it reaches only what the table lists:

```toml
[[guest]]
source.path = "./guests/adapter.wasm"
routes.http = ["/adapter"]

[guest.grants]
env = ["ADAPTER_*"]                    # of the variables [env] inherit passes
mounts = ["data"]                      # the [[mount]]s it sees, by name
hosts = ["api.example.com"]            # the hosts it calls out to
guests = ["ledger"]                    # the guests it calls or loads
interfaces = [                         # the packages it imports code from
    "wasi:cli", "wasi:clocks", "wasi:random", "omnia:otel",  # what every omnia-sdk guest imports
    "wasi:filesystem",                 # to use its mount
    "wasi:http",                       # to serve /adapter and call its hosts
    "acme:ledger",                     # a package it calls `ledger` through
]
```

Once a guest has a table, a list the table leaves out grants nothing, `interfaces` included. Each list is held where the access happens:

- `env` filters the guest's environment, and `mounts` its preopens and the paths it may load a component from, so it never sees what it was not granted.
- `hosts` refuses an outbound `wasi:http` request, or a redirect, to a host outside it before anything connects. It holds a model worker's `egress`, and the registry a plugin load names, the same way. It matches host names alone, so `*` reaches every name and never an address.
- `guests` refuses a link call to a guest outside it before that guest loads, and an `omnia:plugins` load that registers under a name outside it before anything is read.
- `interfaces` refuses a component that imports code from a package outside it when the guest is admitted, naming every such package at once. Name each package rather than writing `wasi:*`, which would grant `wasi:http`, and with it outbound requests to its `hosts`, to a guest that serves no HTTP.

In Rust, `GuestEntry::grants(Grants::none().mounts(["data"]))` declares a table, and `grants: { .. }` does in `runtime!`. Startup logs each guest without one, so what nothing narrows stays visible. A guest loaded at run time through `omnia:plugins/loader` by path or release runs under `[plugins.grants]` within the grants of the guest that loaded it, or under its loader's grants when the deployment declares none. A declared guest a load names keeps its own table. The [security model](../security-model.md#per-guest-grants) has the full rules.

## How execution scales

All guests share one wasmtime engine (the compiler/runtime) and one linker (the table of host functions). Each guest is prepared once at startup. Every inbound request — and every guest-to-guest call — then gets a **new instance**, discarded afterwards. Guests never keep state in memory across calls; put durable state behind a capability such as key-value or SQL.

The pooling allocator (on by default) reuses the instance slots so this stays cheap. Tunables are in [Configuration](../reference/configuration.md#instance-pooling).
