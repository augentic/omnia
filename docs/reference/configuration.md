# Configuration Reference

Omnia is configured entirely through environment variables (runtime options and backend connections) and an optional deployment manifest (guests, routes, mounts, link interfaces). This page lists both.

## Runtime environment variables

### General

| Variable        | Default                                                    | Meaning                                                                                                                                          |
| --------------- | ---------------------------------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------ |
| `RUST_LOG`      | unset (command `info`, server `warn`)                      | Log filter (e.g. `info`, `debug`, `omnia=trace`) for the whole process — the host console and every guest alike. The level is decided once per run: a verbosity flag (below), else this variable, else the mode's default — `info` for a command, `warn` for a server. Unset, a bare server run shows host warnings (a mount that failed to preopen) and nothing else; a bare command run shows the guests' progress too. The server-mode `omnia ready` readiness line is at `info`; the rest of the runtime plumbing (initializing, command-mode ready, guest lifecycle, `wasi:cli/run` bracketing) is at `debug`. Noisy dependencies (`hyper`, `h2`, `tonic`, `opentelemetry`, `opentelemetry_sdk`, `omnia_wasi_otel`) are always muted, and a failed telemetry flush (no collector listening) reports at `debug`. Every guest's WASI environment carries the decided level as its `RUST_LOG` — a flag replaces this variable there, a bare run fills it only when unset — and the process environment itself is never written. See [Verbosity flags](#verbosity-flags). |
| `OTEL_GRPC_URL` | unset (`http://localhost:4317` via OpenTelemetry defaults) | OTLP gRPC endpoint for exporting host traces and metrics (requires the `otlp` feature). Export errors from a missing collector never reach the console — the filter always mutes `opentelemetry` / `opentelemetry_sdk`. |
| `OMNIA_CONFIG`  | unset                                                      | Path to the deployment manifest; the `--config` flag takes precedence.                                                                           |
| `COMPONENT`     | unset                                                      | Overrides the deployment name everywhere it appears — the OpenTelemetry service name, server logs, and the `omnia ready` line; defaults to the deployment name (first guest id). Read once at startup, never written back to the environment. |

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
| `MAX_DISPATCH_DEPTH` | `8`                   | Maximum nesting depth for host-mediated guest-to-guest calls.         |
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

`MEMORY_RESERVATION`, `MEMORY_GUARD_SIZE`, `MEMORY_RESERVATION_FOR_GROWTH`, and `BRANCH_HINTING` affect compilation and must be identical when a component is pre-compiled (`compile`) and later run.

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

Selected by `--config <path>` or `OMNIA_CONFIG`, or compiled in as a default via the `runtime!` macro's `config:` field or inline manifest keys (see [Composing a Runtime](../guides/composing-a-runtime.md#default-manifest-config)). The manifest is sparse: every section is optional except at least one `[[guest]]`, and omitted fields fall back to defaults. All relative paths resolve against the manifest's directory.

The same schema is constructible programmatically as an `omnia::Manifest` value (`Manifest::new()` with the fluent `guest`/`mounts`/`env`/`link`/`locations` setters, or `Manifest::from_wasm` for the one-guest shorthand; routes are set on each `GuestEntry` with its `route_http`/`route_messaging`/`route_websocket` builders) and passed to `DeploymentBuilder::new().manifest(...)` — see [Multi-Guest Deployments](../guides/multi-guest-deployments.md#programmatic-manifests). Either way, the invariants (at least one guest, unique ids, in-process transport) are validated when the deployment is built.

```toml
# --- Host-mediated interfaces (optional, deployment-wide) --------------
[link]
interfaces = ["omnia:link/echo"]    # interfaces the host dispatches between guests

# --- Guests (required, repeatable) -----------------------------------
[[guest]]
id = "router"                       # opaque identity; never parsed by the runtime
source.path = "./router.wasm"       # .wasm or pre-compiled .bin
routes.http = ["/api"]              # inbound routes targeting this guest;
routes.websocket = ["events.*"]     # one optional list per trigger

[[guest]]
id = "responder"
source.path = "./responder.wasm"
routes.messaging = ["events.build.>"]
command = true                      # command-mode target (at most one guest)

# --- Mounts (optional, repeatable) ------------------------------------
[[mount]]
name = "."                          # guest-visible preopen name
path = "../workspace"               # host path
writable = true                     # omit for read-only (default)

# --- Plugin locations (optional, repeatable) --------------------------
[[plugin.location]]
name = "."                          # where guest path loads resolve
path = "./adapters"                 # host directory, opened at startup

[[plugin.location]]
registry = "ghcr.io"                # default registry for package loads (at most one)
config = """                        # optional wasm-pkg client configuration (TOML)
[namespace_registries]
wasi = "wasi.dev"
"""

# --- Transport (optional) ----------------------------------------------
[transport]
default = "in-process"              # in-memory routing; the only implemented kind
```

Field notes:

- **`guest.id`** — opaque to the runtime core; routing and dispatch refer to it.
- **`guest.source`** — `source.path` is implemented; `source.oci` parses but is rejected with "not yet supported".
- **`[link] interfaces`** — deployment-wide host-mediated interfaces, unioned with CLI `--link` values. The host polyfills each onto the shared linker and dispatches calls to whichever guest exports it — including a guest registered after startup. There is no per-guest form: the linker is shared, so a dispatched interface is wired for the whole deployment. A runtime built without the `link` feature refuses a non-empty list at startup.
- **`guest.command`** — marks the guest command mode drives (its `wasi:cli/run`); at most one guest may carry it. Without a mark, the sole `wasi:cli/run` exporter is the catch-all — several unmarked exporters fail the run as ambiguous.
- **`mount`** — preopened into *every* guest sandbox. CLI `--mount` entries layer on top; a duplicate guest-visible name wins over the manifest.
- **`[[plugin.location]]`** — where the `omnia:plugins/loader` acquires packages: `{ name, path }` entries are named roots for path loads (all fold into one `PathMounts`, opened when the runtime assembles), `{ registry, config? }` the registry policy for package references (at most one): the default endpoint and, optionally, a wasm-pkg client configuration as TOML routing namespaces and packages to other registries; an entry mixing the two shapes is a parse error. Only a runtime whose `runtime!` declares a `plugin:` block beside `config:` installs them, and it must be built with omnia's `plugin` feature — a runtime without it refuses a manifest carrying any `[[plugin.location]]` entry at startup; with no entries every load refuses typed.
- **`guest.routes`** — inbound routes targeting the declaring guest, one list per trigger: `http` prefixes (longest prefix wins), `messaging` topics and `websocket` routes (NATS-style: `*` one token, `>` the rest). Route tables are aggregated across guests at load. If a trigger has no routes and exactly one guest exports its handler, that guest is the catch-all. CLI routes are not yet parsed; a sole `wasi:cli/run` exporter receives command-mode invocations.
- **`transport`** — `in-process` (the default) is in-memory routing of lifted values to a fresh callee task. `unix`, `nats`, and `quic` are reserved for distributed dispatch and rejected at load today.
