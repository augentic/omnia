# WASI Interfaces Reference

Every capability Omnia exposes to guests, its interface crate, the zero-config default backend that ships in-tree, and the production backends available in the [`omnia-backends`](https://github.com/augentic/omnia-backends) repository.

## Interface matrix

| Interface | Crate | Host type | Default backend | Production backends |
| --------- | ----- | --------- | --------------- | ------------------- |
| HTTP client/server | `wasi-http` | `WasiHttp` † | `HttpDefault` — hyper outbound client; the trigger's server listens on `HTTP_ADDR` (default `0.0.0.0:8080`) | — |
| Key-value storage | `wasi-keyvalue` | `WasiKeyValue` | `KeyValueDefault` — in-memory, serving the buckets `KEYVALUE_BUCKETS` declares (default `default`) | `omnia-redis`, `omnia-nats` |
| Pub/sub messaging | `wasi-messaging` | `WasiMessaging` † | `MessagingDefault` — in-process broadcast | `omnia-kafka`, `omnia-nats` |
| Blob storage | `wasi-blobstore` | `WasiBlobStore` | `BlobStoreDefault` — in-memory | `omnia-azure-blob`, `omnia-mongodb`, `omnia-nats` |
| SQL | `wasi-sql` | `WasiSql` | `SqlDefault` — SQLite, serving `default` (`SQL_DATABASE`, default shared in-memory) and the databases `SQL_DATABASES` declares | `omnia-postgres` |
| Document store | `wasi-docstore` | `WasiDocStore` | `DocStoreDefault` — in-memory, serving the collections `DOCSTORE_COLLECTIONS` declares (default `default`) | `omnia-azure-table` |
| Configuration | `wasi-config` | `WasiConfig` | `ConfigDefault` — a snapshot of the host variables `CONFIG_KEYS` names (default none) | — |
| Secrets vault | `wasi-vault` | `WasiVault` | `VaultDefault` — in-memory, serving the lockers `VAULT_LOCKERS` declares (default `default`) | `omnia-azure-vault` |
| Identity/OAuth | `wasi-identity` | `WasiIdentity` | `IdentityDefault` — OAuth2 client flow for the identity `default` (`IDENTITY_CLIENT_ID`, `IDENTITY_CLIENT_SECRET`, `IDENTITY_TOKEN_URL`) | `omnia-azure-id` |
| Observability | `wasi-otel` | `WasiOtel` | `OtelDefault` — sends guest telemetry to the host's collector (`OTEL_EXPORTER_OTLP_ENDPOINT`), and drops it when none is set | — |
| WebSockets | `wasi-websocket` | `WasiWebSocket` † | `WebSocketDefault` — tungstenite server (`WEBSOCKET_ADDR`, default `0.0.0.0:80`) | — |
| Model completions | `wasi-model` | `WasiModel` | `ModelDefault` — deterministic echo, serving the models `MODEL_NAMES` declares (default `default`); what an echo cannot honour, such as a schema format, is `unsupported` | `omnia-genai`, `omnia-cursor` |
| Version control | `wasi-vcs` | `WasiVcs` | — none | `omnia-git` |

† Trigger host: runs a server and drives guest instances per inbound request. All other hosts only serve guest-initiated calls.

Notes:

- Package names on crates.io carry the `omnia-` prefix (`omnia-wasi-keyvalue`, and so on); directory names in `crates/` drop it.
- `wasi-keyvalue`, `wasi-blobstore`, `wasi-docstore`, `wasi-messaging`, `wasi-sql`, `wasi-websocket`, `wasi-identity`, `wasi-vault`, `wasi-otel`, and `wasi-config` serve `omnia:keyvalue`, `omnia:blobstore`, `omnia:docstore`, `omnia:messaging`, `omnia:sql`, `omnia:websocket`, `omnia:identity`, `omnia:vault`, `omnia:otel`, and `omnia:config` alone.
- Every `omnia:*` package takes its `error`, its `timestamp`, and its `json` from `omnia:types`, which `wasi-types` holds. Each interface crate copies it into `wit/deps`, and the guest bindings in `omnia-sdk` generate it once, so a guest matches one `Error` whichever interface returned it.
- The unstable interfaces are `consumer`, `log`, and `admin` in `omnia:messaging`, `uploads` and `presign` in `omnia:blobstore`, and `logs` in `omnia:otel`. `omnia-sdk` carries their bindings raw, with no helpers. Every host links them, and answers `unsupported` for each today.
- Most defaults are genuinely zero-config and in-memory. The exceptions: `HttpDefault` and `WebSocketDefault` bind real TCP ports, and `IdentityDefault` needs OAuth credentials. `ModelDefault` is a zero-config echo — it answers text/json completions with the prompt itself but answers `format::schema` with `unsupported`, so deployments bind a real backend and tests script `ScriptedModel` or define inline scenario backends. `wasi-vcs` ships no default at all: a repository is a tree on disk, so a deployment that links `WasiVcs` names a production backend, and a test scripts a `VcsClient` of its own.
- `omnia:vcs` opens a `workspace` handle on the repository or working copy at a `location`, a lent mount-root descriptor plus a relative subpath, and every call goes through the handle. The host resolves the location beneath its mount before any backend runs. A root that is not a mount, a subpath that leads out of one, and a change beneath a read-only mount are `denied`. A subpath that is not plainly spelled, such as one that starts at `/`, is `invalid`. None of these reaches the backend. The backend works through a `Place`, an open directory handle the host holds for the whole call, never a path it would walk again.
- `omnia:model` resolves a completion's `grants.workspace` location the same way, once, as the completion begins. A backend reaches the lent tree through `ToolHost::lend`, a `Lend` holding the open directory and the mount's `writable` bit, never a path it would walk again. A backend that runs processes there binds them to that directory, and lets them write it only beneath a writable mount.
- Each interface crate is the host implementation alone. A guest reaches every interface through the bindings `omnia-sdk` carries, generated from the same WIT.
- Every `omnia:*` package is published to the `omnia.host` wasm-pkg registry. [WIT Packages](wit-packages.md) lists the packages and how a component outside `omnia-sdk` fetches one.

## Crate anatomy

```text
wasi-keyvalue/
├── src/
│   ├── lib.rs            # bindgen!, #[derive(HostCtx)] WasiKeyValue, Host impl
│   ├── bucket_impl.rs    # omnia:keyvalue/bucket Host*WithStore impls on WasiKeyValue, over the table-held BucketHandle
│   ├── store_impl.rs     # omnia:keyvalue/store Host*WithStore impls on WasiKeyValue
│   ├── keys_impl.rs      # ... one file per WIT interface
│   ├── resource.rs       # the Bucket trait a backend implements
│   └── default_impl.rs   # KeyValueDefault backend
└── wit/
    ├── keyvalue.wit    # omnia:keyvalue, owned and edited here
    └── deps/
        └── types.wit   # omnia:types, copied from wasi-types by wit-sync
```

`lib.rs` declares the host type and derives `HostCtx` on it with `#[host_ctx(client = KeyValueClient)]`, which emits the `HasData` and `HostCtx` impls that let a `runtime!` bundle serve it; its view is the library `HostView<'_, dyn KeyValueClient>`, `{ client, table }`. Its `bindgen!` maps `omnia:types` onto `omnia_wasi_types`, so every host returns the one `omnia_wasi_types::Error`, and its `store` flag puts every function in a generated `Host*WithStore` trait. The `*_impl.rs` files implement those on `WasiKeyValue`, and a function borrows the view through `Accessor::with`. See [Extending Omnia](../Architecture.md#extending-omnia) for the chain from backend to linker.

## Supporting crates

| Crate | Purpose |
| ----- | ------- |
| `omnia` | Runtime core: engine, CLI, deployment, registry, dispatch, telemetry |
| `omnia-sdk` | Guest SDK: `Handler`, `Context`, `Handlers`, the HTTP and messaging routers, the command façade (`parse`, `Command`, `Response`), each transport's `export!` macro, errors with their status/exit map, the generated bindings for every `omnia:*` interface with their helpers, and MCP. Features: `http` (default), `command` |
| `omnia-guest-macros` | `#[instrument]`, and the `Settings` and `FromRow` derives |
| `omnia-wasi-types` | `omnia:types`, the WIT types every interface shares |
| `omnia-host-macros` | `runtime!` macro (use via `omnia::runtime!`), `#[derive(HostCtx)]` (use via `omnia_core::HostCtx`, or `omnia::HostCtx` with `crate = omnia`), and `#[derive(FromEnv)]` (use via `omnia_core::FromEnv`, or `omnia::FromEnv` with `crate = omnia`) |
