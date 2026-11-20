# WASI Interfaces Reference

Every capability Omnia exposes to guests, its interface crate, the zero-config default backend that ships in-tree, and the production backends available in the [`omnia-backends`](https://github.com/augentic/omnia-backends) repository.

## Interface matrix

| Interface | Crate | Host type | Default backend | Production backends |
| --------- | ----- | --------- | --------------- | ------------------- |
| HTTP client/server | `wasi-http` | `WasiHttp` † | `HttpDefault` — hyper outbound client, axum inbound server (`HTTP_ADDR`, default `0.0.0.0:8080`) | — |
| Key-value storage | `wasi-keyvalue` | `WasiKeyValue` | `KeyValueDefault` — in-memory, serving the buckets `KEYVALUE_BUCKETS` declares (default `default`) | `omnia-redis`, `omnia-nats` |
| Pub/sub messaging | `wasi-messaging` | `WasiMessaging` † | `MessagingDefault` — in-process broadcast | `omnia-kafka`, `omnia-nats` |
| Blob storage | `wasi-blobstore` | `WasiBlobStore` | `BlobStoreDefault` — in-memory | `omnia-azure-blob`, `omnia-mongodb`, `omnia-nats` |
| SQL | `wasi-sql` | `WasiSql` | `SqlDefault` — SQLite, serving `default` (`SQL_DATABASE`, default shared in-memory) and the databases `SQL_DATABASES` declares | `omnia-postgres` |
| Document store | `wasi-docstore` | `WasiDocStore` | `DocStoreDefault` — in-memory, serving the collections `DOCSTORE_COLLECTIONS` declares (default `default`) | `omnia-azure-table` |
| Configuration | `wasi-config` | `WasiConfig` | `ConfigDefault` — host process environment snapshot | — |
| Secrets vault | `wasi-vault` | `WasiVault` | `VaultDefault` — in-memory, serving the lockers `VAULT_LOCKERS` declares (default `default`) | `omnia-azure-vault` |
| Identity/OAuth | `wasi-identity` | `WasiIdentity` | `IdentityDefault` — OAuth2 client flow for the identity `default` (`IDENTITY_CLIENT_ID`, `IDENTITY_CLIENT_SECRET`, `IDENTITY_TOKEN_URL`) | `omnia-azure-id` |
| Observability | `wasi-otel` | `WasiOtel` | `OtelDefault` — log-only, no export | `omnia-opentelemetry` |
| WebSockets | `wasi-websocket` | `WasiWebSocket` † | `WebSocketDefault` — tungstenite server (`WEBSOCKET_ADDR`, default `0.0.0.0:80`) | — |
| Model completions | `wasi-model` | `WasiModel` | `ModelDefault` — deterministic echo, serving the models `MODEL_NAMES` declares (default `default`); what an echo cannot honour, such as a schema format, is `unsupported` | `omnia-genai`, `omnia-cursor` |
| Version control | `wasi-vcs` | `WasiVcs` | — none | `omnia-git` |

† Trigger host: runs a server and drives guest instances per inbound request. All other hosts only serve guest-initiated calls.

Notes:

- Package names on crates.io carry the `omnia-` prefix (`omnia-wasi-keyvalue`, and so on); directory names in `crates/` drop it.
- `wasi-keyvalue`, `wasi-blobstore`, `wasi-docstore`, `wasi-messaging`, `wasi-sql`, `wasi-websocket`, `wasi-identity`, `wasi-vault`, `wasi-otel`, and `wasi-config` serve `omnia:keyvalue`, `omnia:blobstore`, `omnia:docstore`, `omnia:messaging`, `omnia:sql`, `omnia:websocket`, `omnia:identity`, `omnia:vault`, `omnia:otel`, and `omnia:config` alone.
- Every `omnia:*` package takes its `error`, its `timestamp`, and its `json` from `omnia:types`, which `wasi-types` holds. Each interface crate vendors it under `wit/deps`, and the guest bindings in `omnia-sdk` generate it once, so a guest matches one `Error` whichever interface returned it.
- The unstable interfaces are `consumer`, `log`, and `admin` in `omnia:messaging`, `uploads` and `presign` in `omnia:blobstore`, and `logs` in `omnia:otel`. `omnia-sdk` carries their bindings raw, with no helpers. Every host links them, and answers `unsupported` for each today.
- Most defaults are genuinely zero-config and in-memory. The exceptions: `HttpDefault` and `WebSocketDefault` bind real TCP ports, and `IdentityDefault` needs OAuth credentials. `ModelDefault` is a zero-config echo — it answers text/json completions with the prompt itself but answers `format::schema` with `unsupported`, so deployments bind a real backend and tests script `ScriptedModel` or define inline scenario backends. `wasi-vcs` ships no default at all: a repository is a tree on disk, so a deployment that links `WasiVcs` names a production backend, and a test scripts a `VcsClient` of its own.
- `omnia:vcs` opens a `workspace` handle on the repository or working copy at a `location`, a lent mount-root descriptor plus a relative subpath, and every call goes through the handle. The host resolves the location beneath its mount before any backend runs. A root that is not a mount, a subpath that leads out of one, and a change beneath a read-only mount are `denied`. A subpath that is not plainly spelled, such as one that starts at `/`, is `invalid`. None of these reaches the backend. The backend works through a `Place`, an open directory handle the host holds for the whole call, never a path it would walk again.
- Each interface crate is the host implementation alone. A guest reaches every interface through the bindings `omnia-sdk` carries, generated from the same WIT.

## Crate anatomy

```text
wasi-keyvalue/
├── src/
│   ├── lib.rs          # host-only; empty on wasm32
│   ├── host.rs         # bindgen!, #[derive(HostCtx)] WasiKeyValue, Host impl
│   └── host/           # Host-side implementation (native)
│       ├── bucket_impl.rs    # omnia:keyvalue/bucket Host* impls on the table-held BucketHandle
│       ├── store_impl.rs     # omnia:keyvalue/store Host* impls on HostView<'_, dyn KeyValueClient>
│       ├── keys_impl.rs      # ... one file per WIT interface
│       ├── resource.rs       # the Bucket trait a backend implements
│       └── default_impl.rs   # KeyValueDefault backend
└── wit/
    ├── keyvalue.wit    # omnia:keyvalue, owned and edited here
    └── deps/
        └── types.wit   # omnia:types, vendored from wasi-types
```

`host.rs` declares the host type and derives `HostCtx` on it with `#[host_ctx(client = KeyValueClient)]`, which emits the `HasData` and `HostCtx` impls that let a `runtime!` bundle serve it; its view is the library `HostView<'_, dyn KeyValueClient>` — `{ client, table }` — the `*_impl.rs` files implement the generated `Host*` traits on. See [Extending Omnia](../Architecture.md#extending-omnia) for the chain from backend to linker.

## Supporting crates

| Crate | Purpose |
| ----- | ------- |
| `omnia` | Runtime core: engine, CLI, deployment, registry, dispatch, telemetry |
| `omnia-sdk` | Guest SDK: `Handler`, `Context`, `Handlers`, the HTTP and messaging routers, the command façade (`parse`, `Command`, `Response`), each transport's `export!` macro, errors with their status/exit map, the generated bindings for every `omnia:*` interface with their helpers, and MCP. Features: `http` (default), `command` |
| `omnia-guest-macros` | `#[instrument]`, and the `Settings` and `FromRow` derives |
| `omnia-wasi-types` | `omnia:types`, the WIT types every interface shares |
| `omnia-host-macros` | `runtime!` macro (use via `omnia::runtime!`) and `#[derive(HostCtx)]` (use via `omnia_core::HostCtx`, or `omnia::HostCtx` with `crate = omnia`) |
