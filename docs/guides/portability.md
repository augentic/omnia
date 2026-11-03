# Provider Portability

Omnia separates stateless guest handler intent (`omnia-sdk`) from stateful host service execution (`omnia-backend`). Both layers are free of `wasmtime` and share one set of target-neutral value types. Application handlers bound on `omnia-sdk` capability traits run unchanged across three provider kinds:

1. **Sandboxed WASI component** — `omnia_sdk::WasiProvider` with `omnia_sdk::http!`, `omnia_sdk::messaging!`, and `omnia_sdk::command!` on the Omnia WebAssembly runtime.
2. **Dapr sidecar or decorator composition** — `#[derive(omnia_sdk::Provider)]` combining `omnia_dapr::Dapr<H>` (or `omnia_http_cache::HttpCache<H, S>`) with `WasiProvider`.
3. **In-memory test doubles** — `omnia_test::guest::Provider` for deterministic, sub-millisecond native handler tests.

---

## Capability surface

The SDK defines thirteen capability traits, each implemented by `WasiProvider` on `wasm32-wasip2` and by `omnia_test::guest::Provider` in native handler tests.

| Capability | WASI Host Backing | Test Double | Sidecar / Extension |
| --- | --- | --- | --- |
| `StateStore`, `Publish`, `Config`, `BlobStore` | `omnia-wasi-{keyvalue,messaging,config,blobstore}` | `Memory`, `Sink`, `MapConfig` | `omnia_dapr::Dapr<H>` over any `HttpRequest` |
| `HttpRequest` | `omnia-wasi-http` | `MatchedHttp` | `omnia_http_cache::HttpCache<H, S>` decorator |
| `TableStore` | `omnia-wasi-sql` | `ScriptedTables` | `omnia-orm` query builders |
| `DocumentStore`, `Identity`, `Vault`, `Broadcast` | `omnia-wasi-{docstore,identity,vault,websocket}` | `MemoryDocs`, `FixedIdentity`, `Memory`, `Sink` | — |
| `Model`, `Vcs` | `omnia-wasi-{model,vcs}` over `MountRegistry` | `ScriptedModel`, `ScriptedVcs` | — |
| `Plugins` | `omnia-plugin` (`wasmtime` component loader) | `ScriptedLoader` | — |

> **Count map.** The SDK has thirteen capability traits. `omnia-backend` has thirteen `*Client` service contracts (counterparts to the twelve non-plugin capabilities plus host-only `OtelClient`, which has no handler trait; `Plugins` maps to `omnia_backend::plugins::PackageStore`).

---

## Semantic contract and conformance

Every row of the portable contract is verified by a generic scenario function in `omnia_test::conformance`. A provider test suite invokes `omnia_test::conformance!()`, which fails to compile until every scenario is paired with a test function.

| Capability | Guarantee | Omnia WASI runtime (`WasiProvider`) | Production backends (`omnia-backends`) | `Dapr<H>` (`omnia-dapr`) | Conformance scenario |
| --- | --- | --- | --- | --- | --- |
| `StateStore` get / set / delete | Raw bytes in are raw bytes out; deleting an absent key is a no-op | `wasi:keyvalue/store` | Redis `GET`/`SET`/`DEL`; NATS KV; filesystem | State API (`/v1.0/state/{store}`) with byte-transparent encoding | `conformance::state_get_set_delete` |
| `StateStore` TTL | A value set with `ttl_secs` reads as absent once expired; keys set with `None` persist until deleted | `omnia:keyvalue/ttl` -> `Bucket::set(.., ttl)` | Redis `SET … EX`; filesystem per-key expiry; `KeyValueDefault` per-entry `moka::Expiry` (**Deviation:** NATS KV uses bucket-level `NATS_KV_MAX_AGE_SECS` plus in-process tracking) | `metadata.ttlInSeconds` | `conformance::state_ttl_expires` |
| `StateStore` CAS | Single-step atomic compare-and-swap; `CasError::Conflict` carries the observed value; remaining TTL survives a swap | `wasi:keyvalue/atomics` `swap` -> `Bucket::cas` | Redis Lua CAS with `KEEPTTL`; NATS KV revision update; filesystem & `KeyValueDefault` per-key lock | `GET` + ETag `first-write` `POST` (two sidecar round trips) | `conformance::state_cas` |
| `StateStore` increment | Atomic signed delta on an 8-byte big-endian `i64`; an absent key starts at zero | `wasi:keyvalue/atomics` `increment` | Redis `INCRBY` script; NATS KV revision loop; filesystem & `KeyValueDefault` per-key lock | **Deviation:** bounded CAS retry loop over 8-byte big-endian `i64` (fails under sustained contention) | `conformance::state_increment` |
| `TableStore` query / exec | Parameterized SQL (`$1`, `$2`, ...) over a named connection opened per call | `wasi:sql/readwrite` | `omnia-postgres` pool; `SqlDefault` (`SQLite`) | — | `conformance::table_query_exec` |
| `DocumentStore` | `get`, create-only `insert`, `put` upsert, `delete` -> `bool`, `query` with `Filter`, `order_by`, `limit`, `offset`, `continuation` | `omnia:docstore/store` | `DocStoreDefault` supports full `Filter`; **Deviation:** `omnia-azure-table` rejects `Contains` / `StartsWith` / `EndsWith` (OData has no string functions) | — | `conformance::docstore_crud_query` |
| `BlobStore` | Opaque bytes in are bytes out; missing object is `Ok(None)` on `get`; inclusive `get_range`, metadata, container ops, derived ops (`has`, `delete_objects`, `clear`, `copy_object`, `move_object`) | `wasi:blobstore` | `omnia-azure-blob` (404 -> `Ok(None)`), `omnia-nats`, `omnia-mongodb` (`bson::Binary`), `omnia-filesystem`, `BlobStoreDefault` | **Deviation:** output bindings serve `get`, `put`, `delete`, `list` only; range, metadata, and container ops return `Unsupported` | `conformance::blobstore_objects` |
| `Publish` | At-least-once delivery to the exact topic with headers preserved | `wasi:messaging/producer` | `omnia-kafka`, `omnia-nats`, `MessagingDefault` | Pub/Sub API (`/v1.0/publish/{pubsub}/{topic}`); headers map to `metadata.*` query params | `conformance::publish_send` |
| `Config` | `get(key)` returns the string value, or an error when absent | `wasi:config/store` | `ConfigDefault` (process environment or seeded map) | Configuration API (`/v1.0/configuration/{store}`), with optional process-env override | `conformance::config_get` |
| `Identity` | `token(identity, scopes)` returns `AccessToken { token, expires_in }`; `access_token(identity)` is a defaulted convenience | `omnia:identity/credentials` | `omnia-azure-id`; `IdentityDefault` (`OAuth2`); `IdentityStub` | — | `conformance::identity_token` |
| `Vault` | `get`, `set`, `delete`, `exists`, and `list_ids` over the secret locker | `omnia:vault/vault` | `omnia-azure-vault`; `VaultDefault` | — | `conformance::vault_locker` |
| `HttpRequest` | `fetch(Request<T>)` returns `Response<Bytes>`; a `Client-Cert` header is validated and stripped | `wasi:http` | `HttpDefault` (`reqwest` pool + `Client-Cert` validation) | `WasiProvider` in a guest | `conformance::http_fetch` |
| `Broadcast` | `send(name, data, sockets)` reaches connected WebSocket peers | `omnia:websocket/client` | `WebSocketDefault` | — | `conformance::broadcast_send` |
| `Model` | Enforces `request.validate()`, `ToolSession` call/byte/timeout budgets, and `Workspace` `cap-std` confinement | `omnia:model/completion` | `omnia-genai`, `omnia-cursor`, `ModelDefault` | — | `conformance::model_complete` |
| `Vcs` | Enforces `Place` `cap-std` confinement and read-only mount refusals | `omnia:vcs/{store,workspace,transport}` | `omnia-git` | — | `conformance::vcs_flow` |

### Conformance configurations

The same `omnia_test::conformance` scenarios run across three configurations:

1. **`omnia_test::guest::Provider`** (`crates/omnia-test/tests/conformance.rs`) — verifies that the handler-rung doubles (including `guest::ScriptedVcs`) honour the contract handlers are tested against.
2. **`WasiProvider`** (`crates/test-programs/programs/conformance/portable.rs` driven in `crates/omnia-test/tests/host.rs`) — verifies the sandboxed WASI boundary and proves byte-for-byte interoperability on one shared `KeyValueDefault` bucket.
3. **`Dapr<H>`** (`omnia-dapr` `tests/conformance.rs` over the fake sidecar) — verifies the Dapr HTTP protocol mapping.

---

## Composing providers in a guest

Domain handler crates depend on `omnia-sdk` alone and expose transport routers generic over the provider `P`. Only the guest root crate (`src/lib.rs`) chooses the provider.

### 1. Standard WASI component (`WasiProvider`)

In the guest root crate (`src/lib.rs`), wire the HTTP and messaging routers to `omnia_sdk::WasiProvider` with `omnia_sdk::http!` and `omnia_sdk::messaging!`:

```rust,ignore
pub use self::messaging::messaging_router;
pub use self::routes::router;

omnia_sdk::http!(router(omnia_sdk::WasiProvider));
omnia_sdk::messaging!(messaging_router(omnia_sdk::WasiProvider));
```

Both macros expand to the `#[omnia_wasi_otel::instrument]`-scoped WASI export on `all(target_arch = "wasm32", target_os = "wasi")` and to nothing on native targets, so `cargo nextest` compiles the guest library natively against `omnia_test::guest::Provider` without pulling `wasmtime` or `omnia-wasi-*` into the native dependency graph.

### 2. Overriding capabilities with `#[derive(omnia_sdk::Provider)]`

A guest that overrides one or more capabilities — for example, wrapping outbound HTTP in `omnia_http_cache::HttpCache` or delegating state, pub/sub, and config to a Dapr sidecar via `omnia_dapr::Dapr<WasiProvider>` — derives `Provider` and delegates the remaining capabilities to `WasiProvider`:

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

With `omnia_dapr::Dapr<WasiProvider>`, a WASI component can route `Config`, `StateStore`, and `Publish` through a Dapr HTTP sidecar over `wasi:http` while keeping the remaining capabilities on `WasiProvider`:

```rust,ignore
use omnia_dapr::{Dapr, axum::subscriptions};
use omnia_sdk::WasiProvider;

#[derive(Clone, omnia_sdk::Provider)]
struct DaprGuest {
    #[provider(Config, StateStore, Publish)]
    dapr: Dapr<WasiProvider>,
    #[provider(HttpRequest, Identity, TableStore, DocumentStore, BlobStore, Broadcast)]
    wasi: WasiProvider,
}
```

---

## Internal naming conventions

Each symbol prefix in the workspace names one layer:

- **`Wasi*` (`WasiKeyValue`, `WasiBlobStore`, `WasiSql`, ...)** — `wasmtime` linker hosts in `omnia-wasi-*` and `omnia-plugin`. Never referenced by guest handler code.
- **`Wasi*View<'a>`** — borrowed `(client, table)` store views generated by `omnia_core::wasi_view!`.
- **`WasiProvider`** — the single zero-sized guest provider in `omnia-sdk` implementing every capability trait on `wasm32`.
- **`*Client` (`KeyValueClient`, `SqlClient`, ...)** — engine-free service contracts in `omnia-backend`.
- **`*Default` (`KeyValueDefault`, `SqlDefault`, ...)** — in-memory or process default implementations in `omnia-wasi-*`.
- **Wire constants (`omnia_backend::keyvalue::BUCKET`, `omnia_backend::vault::LOCKER`)** — defined once in `omnia-backend`, re-exported by `omnia-sdk`, and shared by `WasiProvider` and the test harness so keys and secrets land in the same bucket and locker.
- **SDK capability trait names (`StateStore`, `DocumentStore`, `TableStore`, `HttpRequest`, `Publish`, ...)** — kept distinct from Dapr building-block names so handler UFCS never collides with `axum::extract::State`, `docstore::Document`, `sea_query::Table`, or the `http` crate.
