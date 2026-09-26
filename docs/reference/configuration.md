# Configuration Reference

Omnia is configured entirely through environment variables (runtime options and backend connections) and an optional deployment manifest (guests, routes, mounts, registries). This page lists both.

## Runtime environment variables

### General

| Variable        | Default                                                    | Meaning                                                                                                                                          |
| --------------- | ---------------------------------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------ |
| `RUST_LOG`      | unset (command `info`, server `warn`)                      | Log filter (e.g. `info`, `debug`, `omnia=trace`) for the whole process — the host console and every guest alike. The level is decided once per run: a verbosity flag (below), else this variable, else the mode's default — `info` for a command, `warn` for a server. Unset, a bare server run shows host warnings (a mount that failed to preopen) and nothing else; a bare command run shows the guests' progress too. The server-mode `omnia ready` readiness line is at `info`; the rest of the runtime plumbing (initializing, command-mode ready, guest lifecycle, `wasi:cli/run` bracketing) is at `debug`. Noisy dependencies (`hyper`, `h2`, `tonic`, `opentelemetry`, `opentelemetry_sdk`, `omnia_wasi_otel`) are always muted, and a failed telemetry flush (no collector listening) reports at `debug`. Every guest's WASI environment carries the decided level as its `RUST_LOG` — a flag replaces this variable there, a bare run fills it only when unset — and the process environment itself is never written. See [Verbosity flags](#verbosity-flags). |
| `OTEL_GRPC_URL` | unset (`http://localhost:4317` via OpenTelemetry defaults) | OTLP gRPC endpoint for exporting host traces and metrics. Export errors from a missing collector never reach the console — the filter always mutes `opentelemetry` / `opentelemetry_sdk`. |
| `OMNIA_MANIFEST` | unset                                                     | Path to the deployment manifest; the `--manifest` flag takes precedence.                                                                         |
| `COMPONENT`     | unset                                                      | Overrides the program name everywhere it appears — the OpenTelemetry service name, server logs, and the `omnia ready` line; defaults to the crate's package name under `runtime!`, or `omnia`. Read once at startup, never written back to the environment. |

### Verbosity flags

`-v`/`--verbose` and `-q`/`--quiet` select the process tracing level relative to the mode's default: each `-v` is one step up the scale `off`, `error`, `warn`, `info`, `debug`, `trace`, each `-q` one step down, clamped at the ends. The flags repeat (`-vv`, `-qq`); `-v` beside `-q` is a usage error. A flag replaces whatever `RUST_LOG` the process carries, for the host console and every guest; no flag keeps the process variable and falls back to the mode's default only when it is unset, so any filter stays reachable as `RUST_LOG=<filter>` on a bare run.

| Flags   | Command (default `info`) | Server (default `warn`) |
| ------- | ------------------------ | ----------------------- |
| `-qqq`  | `off`                    | `off`                   |
| `-qq`   | `error`                  | `off`                   |
| `-q`    | `warn`                   | `error`                 |
| *(none)* | `info`                  | `warn`                  |
| `-v`    | `debug`                  | `info`                  |
| `-vv`   | `trace`                  | `debug`                 |
| `-vvv`  | `trace`                  | `trace`                 |

On the `run` grammar the flags are the host's, global to the command (`bin -v run …` and `bin run -v …` both parse); past `--` they belong to the guest. On the direct-command path (a `runtime!` in `mode: command` with a compiled-in manifest) argv belongs to the guest verbatim: the host reads `-v`, `-q`, `--verbose`, `--quiet`, and their repetitions (`-vv`) out of argv before `--` to decide the level, and removes nothing, so the guest declares the same flags in its own grammar — `omnia_sdk::api::command::Verbosity`, flattened — for its help and completions to list them and its parser to accept them. An embedder selects the level in code with `DeploymentBuilder::level`.

### Guest limits

| Variable             | Default               | Meaning                                                               |
| -------------------- | --------------------- | --------------------------------------------------------------------- |
| `GUEST_TIMEOUT_MS`   | `30000`               | Wall-clock cap on a single server guest invocation and each link-dispatch hop on a server-rooted chain. A command-mode (`wasi:cli/run`) chain is uncapped, including its link hops. |
| `MAX_MEMORY_BYTES`   | `268435456` (256 MiB) | Maximum linear memory a guest may grow to.                            |
| `MAX_FUEL`           | `0` (off)             | Per-invocation fuel budget; `0` disables metering. Compile-affecting. |
| `MAX_DISPATCH_DEPTH` | `8`                   | Maximum nesting depth for host-mediated calls between guests.         |
| `EPOCH_TICK_MS`      | `10`                  | Epoch-increment interval; the yield granularity for CPU-bound guests. |
| `WASM_BACKTRACE`     | `false`               | Attach guest backtraces to trap errors.                               |
| `ASYNC_STACK_ZEROING` | `false`              | Zero async (fiber) stacks before reuse, so no data lingers between guest invocations that share a recycled stack; costs a wipe per reuse. |

### Instance pooling

The pooling allocator recycles instance slots so per-request instantiation stays cheap. The table below lists the commonly tuned variables; the **complete** list (including the long tail of wasmtime mirrors) lives in `crates/omnia-core/src/options.rs`, where every field carries a doc comment naming its variable and default.

| Variable                     | Default                     | Meaning                                                       |
| ---------------------------- | --------------------------- | ------------------------------------------------------------- |
| `POOLING`                    | `true`                      | Enable the pooling instance allocator.                        |
| `POOL_MAX_INSTANCES`         | `1000`                      | Maximum component instances held by the pool.                 |
| `POOL_MAX_MEMORY_BYTES`      | inherits `MAX_MEMORY_BYTES` | Linear-memory size reserved per pooled memory.                |
| `POOL_MAX_UNUSED_WARM_SLOTS` | `100`                       | Unused warm slots retained for fast reuse.                    |
| `POOL_METRICS_INTERVAL_MS`   | `5000`                      | Interval between pool-occupancy metric samples; `0` disables. |

Further tunables mirror wasmtime's pooling configuration one-to-one: `POOL_TOTAL_CORE_INSTANCES`, `POOL_TOTAL_MEMORIES`, `POOL_TOTAL_TABLES`, `POOL_TOTAL_STACKS`, `POOL_MEMORY_KEEP_RESIDENT`, `POOL_TABLE_KEEP_RESIDENT`, `POOL_ASYNC_STACK_KEEP_RESIDENT`, `POOL_DECOMMIT_BATCH_SIZE`, `POOL_PAGEMAP_SCAN`, per-component/per-module limits, and (with the `mpk` feature) `POOL_MEMORY_PROTECTION_KEYS`. See `crates/omnia-core/src/options.rs` for the authoritative list with doc comments.

### Memory layout and artifacts (compile-affecting)

`MAX_FUEL`, `MEMORY_RESERVATION`, `MEMORY_GUARD_SIZE`, and `BRANCH_HINTING` affect compilation and must be identical when a component is pre-compiled (`compile`) and later run. Together with the two artifact options below they are the runtime's `CompileOptions` (`RuntimeOptions::compile_options()`), the value the compiler takes explicitly: `CompileOptions::default()` is these defaults, so an artifact compiled with it loads into a runtime whose environment sets none of them. `MEMORY_RESERVATION_FOR_GROWTH` is a runtime-only setting.

Two further compile-affecting options trade artifact size against introspection: `DEBUG_SYMBOLS` (default `false`) emits ELF symbol tables in compiled artifacts for profilers and `wasmtime objdump`, and `GENERATE_ADDRESS_MAP` (default `true`) records the machine-code-to-wasm-offset map that gives traps and backtraces their wasm offsets. Set `DEBUG_SYMBOLS=true` when profiling; set `GENERATE_ADDRESS_MAP=false` for the smallest artifacts if you can live without wasm offsets in trap messages.

### Default backends

| Variable                                                             | Default                 | Used by                      |
| -------------------------------------------------------------------- | ----------------------- | ---------------------------- |
| `HTTP_ADDR`                                                          | `0.0.0.0:8080`          | `HttpDefault` inbound server |
| `WEBSOCKET_ADDR`                                                     | `0.0.0.0:80`            | `WebSocketDefault` server    |
| `SQL_DATABASE`                                                       | shared in-memory SQLite | `SqlDefault`                 |
| `IDENTITY_CLIENT_ID`, `IDENTITY_CLIENT_SECRET`, `IDENTITY_TOKEN_URL` | unset                   | `IdentityDefault` OAuth flow |

Production backend variables (Redis, Kafka, Azure, ...) are listed in [Production Backends](../guides/production-backends.md#configuration) and each backend crate's README.

## Deployment manifest (`omnia.toml`)

Selected by `--manifest <path>` or `OMNIA_MANIFEST`, or compiled in as a default via the `runtime!` macro's `manifest:` field or inline manifest keys (see [Composing a Runtime](../guides/composing-a-runtime.md#default-manifest-manifest)). The manifest is sparse: every section is optional except at least one `[[guest]]`, and omitted fields fall back to defaults. All relative paths resolve against the manifest's directory.

The same schema is constructible programmatically as an `omnia::Manifest` value (`Manifest::new()` with the fluent `guest`/`mounts`/`registries` setters, or `Manifest::from_wasm(path)?` for the one-guest shorthand, which reads the component now so it loads at boot; routes are set on each `GuestEntry` with its `route_http`/`route_messaging`/`route_websocket` builders) and passed to `DeploymentBuilder::new().manifest(...)` — see [Multi-Guest Deployments](../guides/multi-guest-deployments.md#programmatic-manifests). Either way, the invariants (at least one guest, every guest named, unique names, in-process transport) are validated when the deployment is built.

```toml
# --- Guests (required, repeatable) -----------------------------------
[[guest]]
name = "router"                     # the guest's identity; never parsed by the runtime
source.path = "./router.wasm"       # .wasm or pre-compiled .bin
routes.http = ["/api"]              # inbound routes targeting this guest;
routes.websocket = ["events.*"]     # one optional list per trigger

[[guest]]
source.path = "./responder.wasm"    # named by the file's stem: `responder`
routes.messaging = ["events.build.>"]
command = true                      # command-mode target (at most one guest)

[[guest]]
name = "tool"                       # loaded when a guest first `load`s "tool"
source.path = "./tool.bin"          # `omnia compile` output loads from a path
digest = "sha256:…"                 # only under a pin; raw .wasm needs none

[[guest]]
source.package = "acme:adapter@1.2.0"   # fetched through [registries] at first use;
                                    # named `acme:adapter` by its reference

# --- Mounts (optional, repeatable) ------------------------------------
[[mount]]
name = "."                          # guest-visible preopen name
path = "../workspace"
writable = true                     # omit for read-only (default)

# --- Registries (optional) --------------------------------------------
[registries]
path = "./wasm-pkg.toml"            # wasm-pkg client configuration routing package sources

# --- Transport (optional) ----------------------------------------------
[transport]
default = "in-process"              # in-memory routing; the only implemented kind
```

When a guest loads: a guest compiled into the runtime — a `runtime!` invocation's embedded `path:`, the component `run <component>` names, a programmatic `GuestEntry::new(name, bytes)` — loads at boot. Every other guest, which is every `[[guest]]` in a manifest file, loads at its first use: the first time a route, the command drive, a link call, a host dispatch, or a guest's `loader.load(declared("name"))` names it. Nothing marks the difference; the source kind is the rule. Boot still validates everything the manifest states (names, routes, mounts, registries), but a missing file, a pin miss, or a refused artifact surfaces when the guest is first named, not at startup.

Field notes:

- **`guest.name`** — the guest's identity (`GuestId`), opaque to the runtime core; routing, link dispatch, and a guest's `loader.load(declared("name"))` refer to it. Omitted, the `source.path` file's stem names the guest (`./guests/echo.wasm` is `echo`), and a `source.package` reference without its version names that guest (`acme:adapter@1.2.0` is `acme:adapter`); give a name when the file is not called what callers dispatch to, two files share a stem, or two versions of one package are declared. Names are unique. The retired `id` key is an unknown key.
- **`guest.source`** — `source.path` is a local component relative to the manifest: raw `.wasm`, or `omnia compile` output when the entry pins its `digest` (unpinned, a pre-compiled file is refused at first use, since it is read while guests run). `source.package` is an exact `namespace:name@version` fetched through `[registries]` at first use, and admits raw `.wasm` alone. A `runtime!` invocation's inline guest embeds its component instead (`path:` — see the [macro reference](runtime-macro.md#guest-entries-pathpackage-name-routes-command-digest)), and embedded bytes load at boot in either format.
- **`guest.digest`** — the `sha256:<hex>` the source's bytes must hash to, checked before wasmtime sees them. For `omnia compile` output from a `source.path` the pin is what admits it; for raw wasm it is a pin like any other — advised for every path whose file could change between boot and first use, in particular one under a directory the deployment also mounts `writable`. A `declared` load takes no digest of its own; the entry's pin is the one that holds.
- **What guests call between themselves** is declared nowhere. Each component says what it imports and exports; every interface outside the runtime's own `wasi:` and `omnia:` namespaces is relayed by the host to whichever guest exports it — a guest registered after startup, or a declared guest the call is the first use of — and an import no guest exports fails at boot. A runtime built without the `link` feature leaves such imports unsatisfied. A `services` key is an unknown key.
- **`guest.command`** — marks the guest command mode drives (its `wasi:cli/run`); at most one guest may carry it. Without a mark, the sole `wasi:cli/run` exporter among the guests loaded at boot is the catch-all — several unmarked exporters fail the run as ambiguous, and a first-use guest is never the catch-all, so a manifest file's command guest carries the mark.
- **`mount`** — preopened into *every* guest sandbox. A read-only mount is also a root the `omnia:plugins/loader` reads a component from when a guest names a path beneath it; a writable mount never is — a component beneath one is refused before it is read — and a writable mount that shares or nests a read-only mount's directory is refused at startup, so what a guest can write, it cannot run. CLI `--mount` entries layer on top; a duplicate guest-visible name wins over the manifest, before any directory opens.
- **`[registries]`** — the [wasm-pkg client configuration](https://github.com/bytecodealliance/wasm-pkg-tools) (`default_registry`, `namespace_registries`, `package_registry_overrides`, per-registry `[registry."..."]` backend settings) a `source.package` guest is fetched through, as a `path` relative to the manifest. A package resolves to its package override, else its namespace's registry, else `default_registry`; a package routed nowhere is refused, naming its namespace, before any registry is dialled. The same routing governs a package a loading guest names itself: the registry such a load may name serves only a namespace the configuration routes nowhere, and a package the configuration routes is fetched from its registry alone. The file is read and parsed when the deployment builds, so a missing or malformed one fails startup, not the first use. A runtime built without the `loader` feature refuses a manifest carrying `[registries]` or a `source.package` guest at startup; a manifest without `[registries]` refuses every package guest's first use, typed.
- **`guest.routes`** — inbound routes targeting the declaring guest, one list per trigger: `http` prefixes (longest prefix wins), `messaging` topics and `websocket` routes (NATS-style: `*` one token, `>` the rest). Route tables are aggregated across guests at load; a route may name a first-use guest, which loads on the first event that matches. If a trigger has no routes and exactly one guest loaded at boot exports its handler, that guest is the catch-all — a first-use guest never is, so a manifest file's guest is reached by its routes, its `command` mark, a link call, or its name, or it never runs. CLI routes are not yet parsed; a sole `wasi:cli/run` exporter receives command-mode invocations.
- **`transport`** — `in-process` (the default) is in-memory routing of lifted values to a fresh callee task. `unix`, `nats`, and `quic` are reserved for distributed dispatch and rejected at load today.
