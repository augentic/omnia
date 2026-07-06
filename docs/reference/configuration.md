# Configuration Reference

Omnia is configured entirely through environment variables (runtime options and backend connections) and an optional deployment manifest (guests, routes, mounts, links). This page lists both.

## Runtime environment variables

### General

| Variable | Default | Meaning |
| -------- | ------- | ------- |
| `RUST_LOG` | unset | Log filter (e.g. `info`, `debug`, `omnia=trace`). Startup logs, including `omnia ready`, are at `info`. |
| `OTEL_GRPC_URL` | unset (`http://localhost:4317` via OpenTelemetry defaults) | OTLP gRPC endpoint for exporting host traces and metrics. No collector running? Silence export errors with `RUST_LOG=...,opentelemetry_sdk=off`. |
| `OMNIA_CONFIG` | unset | Path to the deployment manifest; the `--config` flag takes precedence. |
| `COMPONENT` | derived | Telemetry/component name; defaults to the deployment name (first guest id). |

### Guest limits

| Variable | Default | Meaning |
| -------- | ------- | ------- |
| `GUEST_TIMEOUT_MS` | `30000` | Wall-clock cap on a single guest invocation. |
| `MAX_MEMORY_BYTES` | `268435456` (256 MiB) | Maximum linear memory a guest may grow to. |
| `MAX_FUEL` | `0` (off) | Per-invocation fuel budget; `0` disables metering. Compile-affecting. |
| `MAX_DISPATCH_DEPTH` | `8` | Maximum nesting depth for host-mediated guest-to-guest calls. |
| `EPOCH_TICK_MS` | `10` | Epoch-increment interval; the yield granularity for CPU-bound guests. |
| `WASM_BACKTRACE` | `false` | Attach guest backtraces to trap errors. |

### Instance pooling

The pooling allocator recycles instance slots so per-request instantiation stays cheap.

| Variable | Default | Meaning |
| -------- | ------- | ------- |
| `POOLING` | `true` | Enable the pooling instance allocator. |
| `POOL_MAX_INSTANCES` | `1000` | Maximum component instances held by the pool. |
| `POOL_MAX_MEMORY_BYTES` | inherits `MAX_MEMORY_BYTES` | Linear-memory size reserved per pooled memory. |
| `POOL_MAX_UNUSED_WARM_SLOTS` | `100` | Unused warm slots retained for fast reuse. |
| `POOL_METRICS_INTERVAL_MS` | `5000` | Interval between pool-occupancy metric samples; `0` disables. |

Further tunables mirror wasmtime's pooling configuration one-to-one: `POOL_TOTAL_CORE_INSTANCES`, `POOL_TOTAL_MEMORIES`, `POOL_TOTAL_TABLES`, `POOL_TOTAL_STACKS`, `POOL_MEMORY_KEEP_RESIDENT`, `POOL_TABLE_KEEP_RESIDENT`, `POOL_ASYNC_STACK_KEEP_RESIDENT`, `POOL_DECOMMIT_BATCH_SIZE`, `POOL_PAGEMAP_SCAN`, per-component/per-module limits, and (with the `mpk` feature) `POOL_MEMORY_PROTECTION_KEYS`. See `crates/omnia/src/options.rs` for the authoritative list with doc comments.

### Memory layout (compile-affecting)

`MEMORY_RESERVATION`, `MEMORY_GUARD_SIZE`, `MEMORY_RESERVATION_FOR_GROWTH`, and `BRANCH_HINTING` affect compilation and must be identical when a component is pre-compiled (`compile`) and later run.

### Default backends

| Variable | Default | Used by |
| -------- | ------- | ------- |
| `HTTP_ADDR` | `0.0.0.0:8080` | `HttpDefault` inbound server |
| `WEBSOCKET_ADDR` | `0.0.0.0:80` | `WebSocketDefault` server |
| `SQL_DATABASE` | shared in-memory SQLite | `SqlDefault` |
| `DOCSTORE_DATABASE` | temp-dir PoloDB file | `DocStoreDefault` |
| `MODEL_REPLAY_DIR` | unset | `ModelDefault` fixture directory |
| `IDENTITY_CLIENT_ID`, `IDENTITY_CLIENT_SECRET`, `IDENTITY_TOKEN_URL` | unset | `IdentityDefault` OAuth flow |

Production backend variables (Redis, Kafka, Azure, ...) are listed in [Production Backends](../guides/production-backends.md#configuration) and each backend crate's README.

## Deployment manifest (`omnia.toml`)

Selected by `--config <path>` or `OMNIA_CONFIG`. The manifest is sparse: every section is optional except at least one `[[guest]]`, and omitted fields fall back to defaults. All relative paths resolve against the manifest's directory.

```toml
# --- Guests (required, repeatable) -----------------------------------
[[guest]]
id = "router"                       # opaque identity; never parsed by the runtime
source.path = "./router.wasm"       # .wasm or pre-compiled .bin
link = ["omnia:link/echo"]          # host-mediated imports this guest may call

[[guest]]
id = "responder"
source.path = "./responder.wasm"

# --- Mounts (optional, repeatable) ------------------------------------
[[mount]]
name = "."                          # guest-visible preopen name
path = "../workspace"               # host path
writable = true                     # omit for read-only (default)

# --- Routes (optional, one table per trigger) --------------------------
[[route.http]]
prefix = "/api"                     # longest prefix wins
guest = "router"

[[route.messaging]]
topic = "events.build.>"            # NATS-style: `*` one token, `>` the rest
guest = "responder"

[[route.websocket]]
route = "events.*"                  # same pattern syntax, spelled `route`
guest = "router"

# --- Transport (optional) ----------------------------------------------
[transport]
default = "in-process"              # the only implemented transport
```

Field notes:

- **`guest.id`** — opaque to the runtime core; routing and linking refer to it.
- **`guest.source`** — `source.path` is implemented; `source.oci` parses but is rejected with "not yet supported".
- **`guest.link`** — interfaces the host polyfills onto the shared linker and dispatches to whichever guest exports them. Unioned with CLI `--link` values.
- **`mount`** — preopened into *every* guest sandbox. CLI `--mount` entries layer on top; a duplicate guest-visible name wins over the manifest.
- **`route.*`** — if a trigger has no routes and exactly one guest exports its handler, that guest is the catch-all. `[[route.cli]]` is not yet parsed; a sole `wasi:cli/run` exporter receives command-mode invocations.
- **`transport`** — `unix`, `nats`, and `quic` are reserved for distributed dispatch and rejected at load today.
