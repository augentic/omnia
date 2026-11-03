# Provider Portability

Omnia separates stateless handler intent (`omnia-sdk`) from stateful service execution (`omnia-backend`). Both layers are free of `wasmtime` and share one set of target-neutral value types. Application handlers bound on `omnia-sdk` capability traits deploy unchanged across four provider kinds:

1. **Sandboxed WASI component** — `omnia_sdk::WasiProvider` with `omnia_sdk::http!`, `omnia_sdk::messaging!`, and `omnia_sdk::command!` on the Omnia runtime.
2. **Native binary** — `#[derive(omnia_sdk::Provider)]` over `omnia_native::Direct<T>` and `omnia_native::Serve`, with zero WebAssembly engine in the dependency tree.
3. **Dapr sidecar app** — `#[derive(omnia_sdk::Provider)]` combining `omnia_dapr::Dapr<H>` for Dapr building blocks with `Direct<T>` for capabilities Dapr does not serve.
4. **In-memory test doubles** — `omnia_test::guest::Provider` for deterministic, sub-millisecond handler tests.

---

## Portable surface

The SDK defines thirteen capability traits. Twelve are portable across `WasiProvider`, `Direct<T>`, and test doubles; only `Plugins` requires the `wasmtime` component engine.

| Capability | Status | Backing |
| --- | --- | --- |
| `StateStore`, `TableStore`, `DocumentStore`, `BlobStore`, `Publish`, `Config`, `Identity`, `Vault`, `HttpRequest`, `Broadcast` | Portable (all providers) | `WasiProvider`, `Direct<T>`, `omnia_test::guest::Provider`, and `Dapr<H>` where Dapr has a building block |
| `Model`, `Vcs` | Portable (`WasiProvider`, `Direct<T>`, test doubles) | `ToolSession` budgets and `cap-std` `Workspace` / `Place` confinement live in `omnia-backend`, so `Direct::with_mounts` enforces the same mount and session rules as the WASI host |
| `Plugins` | Engine-only (`WasiProvider`, test doubles) | Loading and instantiating a `.wasm` component requires the `wasmtime` engine (`omnia-plugin`) |

> **Count map.** The SDK has thirteen capability traits (the twelve portable traits plus engine-only `Plugins`). `omnia-backend` has thirteen `*Client` traits (counterparts to the twelve portable capabilities plus host-only `OtelClient`, which has no handler trait; `Plugins` maps to `omnia_backend::plugins::PackageStore`).

---

## Semantic contract and conformance

Every row of the portable contract is verified by a generic scenario function in `omnia_test::conformance`. A provider test suite invokes `omnia_test::conformance!()`, which fails to compile until every scenario is paired with a test function.

| Capability | Guarantee | Omnia runtime (`WasiProvider`) | `Direct<production>` | `Dapr<H>` | Conformance scenario |
| --- | --- | --- | --- | --- | --- |
| `StateStore` get / set / delete | Raw bytes in are raw bytes out; deleting an absent key is a no-op | `wasi:keyvalue/store` | Redis `GET`/`SET`/`DEL`; NATS KV; filesystem | State API (`/v1.0/state/{store}`) with byte-transparent encoding | `conformance::state_get_set_delete` |
| `StateStore` TTL | A value set with `ttl_secs` reads as absent once expired; keys set with `None` persist until deleted | `omnia:keyvalue/ttl` -> `Bucket::set(.., ttl)` | Redis `SET … EX`; NATS KV `max_age`; `KeyValueDefault` per-entry `moka::Expiry` | `metadata.ttlInSeconds` | `conformance::state_ttl_expires` |
| `StateStore` CAS | Single-step atomic compare-and-swap; `CasError::Conflict` carries the observed value; remaining TTL survives a swap | `wasi:keyvalue/atomics` `swap` -> `Bucket::cas` | Redis Lua CAS with `KEEPTTL`; NATS KV revision update; `KeyValueDefault` per-key lock | `GET` + ETag `first-write` `POST` (two sidecar round trips) | `conformance::state_cas` |
| `StateStore` increment | Atomic signed delta on an 8-byte big-endian `i64`; an absent key starts at zero | `wasi:keyvalue/atomics` `increment` | Redis `INCRBY` script; `KeyValueDefault` per-key lock | **Deviation:** bounded CAS retry loop over 8-byte big-endian `i64` (fails under sustained contention) | `conformance::state_increment` |
| `TableStore` query / exec | Parameterized SQL (`$1`, `$2`, ...) over a named connection opened per call | `wasi:sql/readwrite` | `omnia-postgres` pool; `SqlDefault` (`SQLite`) | No building block (`Direct` slot) | `conformance::table_query_exec` |
| `DocumentStore` | `get`, create-only `insert`, `put` upsert, `delete` -> `bool`, `query` with `Filter`, `order_by`, `limit`, `offset`, `continuation` | `omnia:docstore/store` | `DocStoreDefault` supports full `Filter`; **Deviation:** `omnia-azure-table` rejects `Contains` / `StartsWith` / `EndsWith` (OData has no string functions) | No building block (`Direct` slot) | `conformance::docstore_crud_query` |
| `BlobStore` | Opaque bytes in are bytes out; missing object is `Ok(None)` on `get`; inclusive `get_range`, metadata, container ops, `BlobStoreExt` | `wasi:blobstore` | `omnia-azure-blob` (404 -> `Ok(None)`), `omnia-nats`, `omnia-mongodb` (`bson::Binary`), `omnia-filesystem`, `BlobStoreDefault` | **Deviation:** output bindings serve `get`, `put`, `delete`, `list` only; range, metadata, and container ops return `Unsupported` | `conformance::blobstore_objects` |
| `Publish` | At-least-once delivery to the exact topic with headers preserved | `wasi:messaging/producer` | `omnia-kafka`, `omnia-nats`, `MessagingDefault` | Pub/Sub API (`/v1.0/publish/{pubsub}/{topic}`); headers map to `metadata.*` query params | `conformance::publish_send` |
| `Config` | `get(key)` returns the string value, or an error when absent | `wasi:config/store` | `ConfigDefault` (process environment or seeded map) | Configuration API (`/v1.0/configuration/{store}`), with optional process-env override | `conformance::config_get` |
| `Identity` | `token(identity, scopes)` returns `AccessToken { token, expires_in }`; `access_token(identity)` is a defaulted convenience | `omnia:identity/credentials` | `omnia-azure-id`; `IdentityDefault` (`OAuth2`); `IdentityStub` | No building block (`Direct` slot; Dapr Secrets returns static secrets, not tokens) | `conformance::identity_token` |
| `Vault` | `get`, `set`, `delete`, `exists`, and `list_ids` over the secret locker | `omnia:vault/vault` | `omnia-azure-vault`; `VaultDefault` | Dapr Secrets API (read-only) or `Direct<omnia_azure_vault::Client>` | `conformance::vault_locker` |
| `HttpRequest` | `fetch(Request<T>)` returns `Response<Bytes>`; a `Client-Cert` header is validated and stripped | `wasi:http` | `HttpDefault` (`reqwest` pool + `Client-Cert` validation) | `Direct<HttpDefault>` natively, `WasiProvider` in a guest | `conformance::http_fetch` |
| `Broadcast` | `send(name, data, sockets)` reaches connected WebSocket peers | `omnia:websocket/client` | `WebSocketDefault` | No building block (`Direct` slot) | `conformance::broadcast_send` |
| `Model` | Enforces `request.validate()`, `ToolSession` call/byte/timeout budgets, and `Workspace` `cap-std` confinement | `omnia:model/completion` | `Direct<omnia_genai::Client>`, `Direct<omnia_cursor::Client>`, `Direct<ModelDefault>` over `MountRegistry` | No building block (`Direct` slot) | `conformance::model_complete` |
| `Vcs` | Enforces `Place` `cap-std` confinement and read-only mount refusals | `omnia:vcs/{store,workspace,transport}` | `Direct<omnia_git::Client>` over `MountRegistry` | No building block (`Direct` slot) | `conformance::vcs_flow` |

### Conformance configurations

The same `omnia_test::conformance` scenarios run across five configurations:

1. **`omnia_test::guest::Provider`** (`crates/omnia-test/tests/conformance.rs`) — verifies that the handler-rung doubles honour the contract handlers are tested against.
2. **`Direct<*Default>`** (`crates/omnia-native/tests/conformance.rs`) — verifies the in-process bridge over every in-memory and process default.
3. **`WasiProvider`** (`crates/test-programs/programs/conformance/portable.rs` driven in `crates/omnia-native/tests/conformance.rs`) — verifies the sandboxed WASI boundary and proves byte-for-byte interoperability between `WasiProvider` and `Direct<KeyValueDefault>` on one shared `KeyValueDefault` instance.
4. **`Direct<production>`** (`omnia-backends` `tests/conformance.rs`, `#[ignore]`-gated on live service credentials) — verifies production backends and pins documented deviations.
5. **`Dapr<Direct<HttpDefault>>`** (`omnia-dapr` `tests/conformance.rs` over the fake sidecar) — verifies the Dapr HTTP protocol mapping.

---

## Switching providers at the binary entrypoint

Domain handler crates depend on `omnia-sdk` alone and expose transport routers generic over the provider `P`. Only the binary entrypoint chooses the provider.

### 1. Sandboxed WASI component (`WasiProvider`)

In the guest root crate (`src/lib.rs`), wire the HTTP and messaging routers to `omnia_sdk::WasiProvider` with `omnia_sdk::http!` and `omnia_sdk::messaging!`:

```rust,ignore
pub use self::messaging::messaging_router;
pub use self::routes::router;

omnia_sdk::http!(router(omnia_sdk::WasiProvider));
omnia_sdk::messaging!(messaging_router(omnia_sdk::WasiProvider));
```

Both macros expand to the `#[omnia_wasi_otel::instrument]`-scoped WASI export on `all(target_arch = "wasm32", target_os = "wasi")` and to nothing on native targets.

A guest that overrides a single capability (for example, wrapping outbound HTTP in `omnia_http_cache::HttpCache`) derives `Provider` and delegates the remaining capabilities to `WasiProvider`:

```rust,ignore
#[derive(Clone, Default, omnia_sdk::Provider)]
struct Cached {
    #[provider(HttpRequest)]
    http: HttpCache<WasiProvider, WasiProvider>,
    #[provider(Config, Identity, Publish, Broadcast, StateStore, BlobStore, DocumentStore, TableStore)]
    wasi: WasiProvider,
}

omnia_sdk::http!(router(Cached::default()));
```

### 2. Native binary (`omnia_native::Direct` and `Serve`)

A native entrypoint connects the service clients it needs, wraps each in `Direct::new` (or `Direct::with_mounts` when lending directories to `Model` or `Vcs`), and drives the routers with `omnia_native::Serve`:

```rust,ignore
use omnia_backend::{ConfigDefault, Connect as _, HttpDefault, WebSocketDefault};
use omnia_native::{Direct, Serve};

#[derive(Clone, omnia_sdk::Provider)]
struct Native {
    #[provider(Config)]
    config: Direct<ConfigDefault>,
    #[provider(HttpRequest)]
    http: Direct<HttpDefault>,
    #[provider(Identity)]
    identity: Direct<omnia_azure_id::Client>,
    #[provider(Publish)]
    publish: Direct<omnia_kafka::Client>,
    #[provider(StateStore)]
    state: Direct<omnia_redis::Client>,
    #[provider(BlobStore)]
    blob: Direct<omnia_azure_blob::Client>,
    #[provider(DocumentStore)]
    docs: Direct<omnia_azure_table::Client>,
    #[provider(TableStore)]
    tables: Direct<omnia_postgres::Client>,
    #[provider(Broadcast)]
    broadcast: Direct<WebSocketDefault>,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let kafka = omnia_kafka::Client::connect().await?;
    let provider = Native {
        config: Direct::new(ConfigDefault::connect().await?),
        http: Direct::new(HttpDefault::connect().await?),
        identity: Direct::new(omnia_azure_id::Client::connect().await?),
        publish: Direct::new(kafka.clone()),
        state: Direct::new(omnia_redis::Client::connect().await?),
        blob: Direct::new(omnia_azure_blob::Client::connect().await?),
        docs: Direct::new(omnia_azure_table::Client::connect().await?),
        tables: Direct::new(omnia_postgres::Client::connect().await?),
        broadcast: Direct::new(WebSocketDefault::connect().await?),
    };
    Serve::new()
        .http("0.0.0.0:8080", guest::router(provider.clone()))
        .messaging(kafka, guest::messaging_router(provider).await)
        .run()
        .await
}
```

### 3. Kubernetes with Dapr (`omnia_dapr::Dapr`)

A Dapr entrypoint delegates `Config`, `StateStore`, and `Publish` (and optionally basic `BlobStore` operations) to `Dapr<H>`, fills capabilities outside Dapr's building blocks with `Direct<T>`, and merges `omnia_dapr::axum::subscriptions` into the Axum router:

```rust,ignore
use omnia_backend::{Connect as _, HttpDefault, WebSocketDefault};
use omnia_dapr::{Dapr, axum::subscriptions};
use omnia_native::Direct;

#[derive(Clone, omnia_sdk::Provider)]
struct App {
    #[provider(Config, StateStore, Publish)]
    dapr: Dapr<Direct<HttpDefault>>,
    #[provider(HttpRequest)]
    http: Direct<HttpDefault>,
    #[provider(Identity)]
    identity: Direct<omnia_azure_id::Client>,
    #[provider(TableStore)]
    tables: Direct<omnia_postgres::Client>,
    #[provider(DocumentStore)]
    docs: Direct<omnia_azure_table::Client>,
    #[provider(BlobStore)]
    blob: Direct<omnia_azure_blob::Client>,
    #[provider(Broadcast)]
    broadcast: Direct<WebSocketDefault>,
}
```

---

## Internal naming conventions

Each symbol prefix in the workspace names one layer:

- **`Wasi*` (`WasiKeyValue`, `WasiBlobStore`, `WasiSql`, ...)** — `wasmtime` linker hosts in `omnia-wasi-*` and `omnia-plugin`. Never referenced by guest or native handler code.
- **`Wasi*View<'a>`** — borrowed `(client, table)` store views generated by `omnia_core::wasi_view!`.
- **`WasiProvider`** — the single zero-sized guest provider in `omnia-sdk` implementing every capability trait on `wasm32` (`WasiModel`, `WasiVcs`, and `WasiPlugins` in `omnia-sdk` are deprecated aliases for `WasiProvider`).
- **`*Client` (`KeyValueClient`, `SqlClient`, ...)** — engine-free service contracts in `omnia-backend`.
- **`*Default` (`KeyValueDefault`, `SqlDefault`, ...)** — in-memory or process default implementations in `omnia-backend`.
- **`Direct::new(client)` / `Direct::with_mounts(client, mounts)`** — associated constructors on `omnia_native::Direct<T>`.
- **Wire constants (`omnia_backend::keyvalue::BUCKET`, `omnia_backend::vault::LOCKER`)** — defined once in `omnia-backend`, re-exported by `omnia-sdk` and `omnia-native`, and shared by `WasiProvider`, `Direct<T>`, and the test harness so keys and secrets land in the same bucket and locker across every deployment mode.
- **SDK capability trait names (`StateStore`, `DocumentStore`, `TableStore`, `HttpRequest`, `Publish`, ...)** — kept distinct from Dapr building-block names so handler UFCS never collides with `axum::extract::State`, `docstore::Document`, `sea_query::Table`, or the `http` crate.

---

## Source-level vs. binary-level portability

Omnia provides **source-level portability**: handler crates compile unchanged into a sandboxed WASI component, a native container or bare-metal binary, a Dapr application, or an AWS Lambda / Cloud Run HTTP service (`axum::Router` is a standard `tower::Service`).

**Binary-level portability** of a compiled `.wasm` artifact to non-Omnia component hosts (such as Spin, wasmCloud, or `wasmtime serve`) is bounded by the WIT packages the component imports and exports:

- `wasi:config@0.2.0-rc.1`, `wasi:blobstore@0.2.0-draft`, `wasi:messaging@0.2.0-draft`, and `wasi:keyvalue@0.2.0-draft2` (without TTL) are shared `wasi-cloud-core` drafts.
- `wasi:http@0.3.0` (via `wasip3`) requires Preview 3 HTTP support on the target host.
- `omnia:keyvalue/ttl`, `omnia:docstore`, `wasi:sql`, and `omnia:{identity,vault,websocket,otel,model,vcs,plugins}` are Omnia-specific interfaces; guests using those capabilities deploy either on the Omnia runtime or natively via `Direct<T>`.
