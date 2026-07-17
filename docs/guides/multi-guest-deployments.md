# Multi-Guest Deployments

A single runtime can host many guests, route inbound traffic between them, preopen host directories into their sandboxes, and let one guest call another through the host. All of this is deployment configuration in a TOML manifest — no host or guest code changes.

Passing a single `.wasm` path to `run` remains the zero-config shorthand; the manifest takes over when you need more than one guest, routes, or mounts.

## The deployment manifest (`omnia.toml`)

Point the runtime at a manifest with `--config` (or the `OMNIA_CONFIG` environment variable):

```bash
cargo run --example http-routing -- run --config examples/http-routing/omnia.toml
```

A runtime can also compile in a default manifest path with the `runtime!` macro's `config:` field — used only when the command line supplies no source (see [Composing a Runtime](composing-a-runtime.md#default-manifest-config)).

A manifest declares guests, mounts, routes, and (eventually) transports. Every field is optional except at least one `[[guest]]`. Paths resolve relative to the manifest's own directory.

```toml
[[guest]]
id = "api"                              # opaque identity; the runtime never parses it
source.path = "./guests/api.wasm"       # .wasm or pre-compiled .bin

[[guest]]
id = "admin"
source.path = "./guests/admin.wasm"

[[route.http]]
prefix = "/admin"
guest = "admin"

[[route.http]]
prefix = "/"
guest = "api"
```

The full field reference lives in [Configuration](../reference/configuration.md#deployment-manifest-omniatoml).

## Routing inbound traffic

Each trigger has its own route table, independent of which guests are loaded:

- **`[[route.http]]`** — `prefix` matched by longest path prefix. One HTTP server fronts all guests.
- **`[[route.messaging]]`** — `topic` matched by NATS-style pattern (`.`-separated tokens, `*` matches one token, `>` matches the rest).
- **`[[route.websocket]]`** — same pattern syntax, spelled `route`.

If a trigger has no routes and exactly one guest exports its handler, that guest is the catch-all — so single-guest deployments need no route tables at all.

The [`http-routing`](../../examples/http-routing/) example runs two HTTP guests behind `/a` and `/b` prefixes.

## Mounts: giving guests a workspace

Guests run in a sandbox with no filesystem access by default. A **mount** preopens a host directory into every guest's sandbox under a guest-visible name:

```toml
[[mount]]
name = "."          # what the guest sees in preopens.get-directories()
path = "../.."      # host path, relative to the manifest
writable = true     # omit for read-only (the default)
```

The equivalent on the command line (repeatable, layered over the manifest, last-wins per name):

```bash
cargo run --example model -- run guest.wasm --mount path=workspace,name=.,writable
```

Guests discover mounts through `wasi:filesystem/preopens`:

```rust
let directories = preopens::get_directories();
let workspace = directories.iter().find_map(|(dir, name)| (name == ".").then_some(dir));
```

The [`model`](../../examples/model/) example lends a mounted workspace to a model backend this way.

## Guest-to-guest linking

One guest can import an interface that another guest exports, with the host mediating the call. The importing guest names the interface in its `link` allow-list:

```toml
[[guest]]
id = "responder"
source.path = "./responder.wasm"        # exports omnia:link/echo

[[guest]]
id = "router"
source.path = "./router.wasm"           # imports omnia:link/echo
link = ["omnia:link/echo"]
```

At startup, the runtime polyfills each linked interface onto the shared linker and dispatches calls to whichever guest exports it, over an in-process channel. The runtime sees only opaque interface strings and guest identities — no domain knowledge lives in the core (this is the glossary's [Law 2](../glossary.md#law-2)).

Notes:

- `--link <interface>` on the command line unions with the manifest's per-guest lists.
- Nested dispatch depth is bounded by `MAX_DISPATCH_DEPTH` (default 8) to catch accidental recursion.
- Only the in-process transport is implemented; declaring `unix`, `nats`, or `quic` under `[transport]` is rejected at load.

The [`guest-link`](../../examples/guest-link/) example is a complete router/responder pair.

## How execution scales

All guests share one wasmtime engine and linker, and each is pre-instantiated once at startup. Every inbound request or dispatched call then instantiates a fresh instance in its own store, so guests never share state within or across requests. The pooling allocator (on by default) recycles instance slots to keep per-request cost low — tunables are listed in [Configuration](../reference/configuration.md#instance-pooling).
