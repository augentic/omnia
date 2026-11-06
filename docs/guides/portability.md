# Provider Portability

Omnia runs WASI components. A guest's handlers reach the host through the `omnia-sdk` capability traits, and `omnia_sdk::WasiProvider` is the provider that serves every one of them inside the sandbox, over the `wasi:*` and `omnia:*` interfaces. What a capability does on the host side is decided by the backend the embedder binds to that interface: an in-memory default from the `omnia-wasi-*` crate, or a production backend from [`omnia-backends`](https://github.com/augentic/omnia-backends) implementing the same `omnia-backend` `*Client` contract. The guest is untouched either way.

Because a handler is generic over its provider, the same handler code also runs outside the sandbox:

- natively, in a handler's own tests, over the `omnia_test::guest::Provider` doubles;
- composed, when a guest overrides a capability with `#[derive(omnia_sdk::Provider)]` — such as wrapping outbound HTTP in `omnia_http_cache::HttpCache<H, S>` — and delegates the rest to `WasiProvider`.

This guide is the contract those providers share, the suite that checks it, and the shapes a guest uses to choose one.

---

## Capability surface

The SDK defines thirteen capability traits, each implemented by `WasiProvider` on `wasm32-wasip2` and by `omnia_test::guest::Provider` in native handler tests.

| Capability | WASI Host Backing | Test Double | Composition |
| --- | --- | --- | --- |
| `StateStore`, `Publish`, `Config`, `BlobStore` | `omnia-wasi-{keyvalue,messaging,config,blobstore}` | `Memory`, `Sink`, `MapConfig` | `omnia-extensions` decorators / adapters |
| `HttpRequest` | `omnia-wasi-http` | `MatchedHttp` | `omnia_http_cache::HttpCache<H, S>` decorator |
| `TableStore` | `omnia-wasi-sql` | `ScriptedTables` | `omnia-orm` query builders |
| `DocumentStore`, `Identity`, `Vault`, `Broadcast` | `omnia-wasi-{docstore,identity,vault,websocket}` | `MemoryDocs`, `FixedIdentity`, `Memory`, `Sink` | — |
| `Model`, `Vcs` | `omnia-wasi-{model,vcs}` over `MountRegistry` | `ScriptedModel`, `ScriptedVcs` | — |
| `Plugins` | `omnia-plugin` (`wasmtime` component loader) | `ScriptedLoader` | — |

Each host behind a `WasiProvider` capability dispatches to one `omnia-backend` service contract (`KeyValueClient`, `SqlClient`, ...). The contract crate is free of `wasmtime`, so a production backend implements it without linking the runtime; the host crate owns the WIT binding and the in-memory default.

---

## Semantic contract and conformance

Every row of the portable contract is verified by a generic scenario function in `omnia_test::conformance`. A provider test suite invokes `omnia_test::conformance!(...)`, which fails to compile until every selected scenario is paired with a test function.

| Capability | Guarantee | Omnia WASI runtime (`WasiProvider`) | Production backends (`omnia-backends`) | Conformance scenario |
| --- | --- | --- | --- | --- |
| `StateStore` get / set / delete | Raw bytes in are raw bytes out; deleting an absent key is a no-op; a key set without a TTL stays until deleted | `wasi:keyvalue/store` on the `state` bucket | Redis `GET`/`SET`/`DEL`; NATS KV; filesystem | `conformance::state_get_set_delete` |
| `StateStore` TTL | A value set with `ttl_secs` reads as absent once expired | `omnia:keyvalue/ttl` -> `Bucket::set(.., ttl)` | Redis `SET … EX`; NATS per-message TTL (`Nats-TTL`, server 2.11+); filesystem per-key expiry; `KeyValueDefault` per-entry `moka::Expiry` | `conformance::state_ttl_expires` |
| `StateStore` CAS | Single-step atomic compare-and-swap; `CasError::Conflict` carries the observed value; remaining TTL survives a swap | `wasi:keyvalue/atomics` `swap` -> `Bucket::cas` | Redis Lua CAS with `KEEPTTL`; NATS KV revision-conditioned write re-sending the remaining TTL; filesystem & `KeyValueDefault` per-key lock | `conformance::state_cas` |
| `StateStore` increment | Atomic signed delta on an 8-byte big-endian `i64`; an absent key starts at zero | `wasi:keyvalue/atomics` `increment` | Redis `INCRBY`; NATS KV revision loop; filesystem & `KeyValueDefault` per-key lock | `conformance::state_increment` |
| `TableStore` query / exec | Parameterized SQL (`$1`, `$2`, ...) over a named connection opened per call | `wasi:sql/readwrite` | `omnia-postgres` pool; `SqlDefault` (`SQLite`) | `conformance::table_query_exec` |
| `DocumentStore` | `get`, create-only `insert`, `put` upsert, `delete` -> `bool`, `query` with `Filter`, `order_by`, `limit`, `offset`, `continuation` | `omnia:docstore/store` | `DocStoreDefault` supports full `Filter`; **Deviation:** `omnia-azure-table` rejects `Contains` / `StartsWith` / `EndsWith` (OData has no string functions) | `conformance::docstore_crud_query` |
| `BlobStore` | Opaque bytes in are bytes out; missing object is `Ok(None)` on `get`; inclusive `get_range`, metadata, container ops, derived ops (`has`, `delete_objects`, `clear`, `copy_object`, `move_object`) | `wasi:blobstore` | `omnia-azure-blob` (404 -> `Ok(None)`), `omnia-nats`, `omnia-mongodb` (`bson::Binary`), `omnia-filesystem`, `BlobStoreDefault` | `conformance::blobstore_objects` |
| `Publish` | At-least-once delivery to the exact topic with headers preserved | `wasi:messaging/producer` | `omnia-kafka`, `omnia-nats`, `MessagingDefault` | `conformance::publish_send` |
| `Config` | `get(key)` returns the string value, or an error when absent | `wasi:config/store` | `ConfigDefault` (process environment or seeded map) | `conformance::config_get` |
| `Identity` | `token(identity, scopes)` returns `AccessToken { token, expires_in }`; `access_token(identity)` is a defaulted convenience | `omnia:identity/credentials` | `omnia-azure-id`; `IdentityDefault` (`OAuth2`); `IdentityStub` | `conformance::identity_token` |
| `Vault` | `get`, `set`, `delete`, `exists`, and `list_ids` over the secret locker | `omnia:vault/vault` | `omnia-azure-vault`; `VaultDefault` | `conformance::vault_locker` |
| `HttpRequest` | `fetch(Request<T>)` returns `Response<Bytes>`; a `Client-Cert` header is validated and stripped | `wasi:http` | `HttpDefault` (`reqwest` pool + `Client-Cert` validation); no `*Client` contract, the host is the client | `conformance::http_fetch` |
| `Broadcast` | `send(name, data, sockets)` reaches connected WebSocket peers | `omnia:websocket/client` | `WebSocketDefault` | `conformance::broadcast_send` |
| `Model` | Enforces `request.validate()`, `ToolSession` call/byte/timeout budgets, and `Workspace` `cap-std` confinement | `omnia:model/completion` | `omnia-genai`, `omnia-cursor`, `ModelDefault` | `conformance::model_complete` |
| `Vcs` | Enforces `Place` `cap-std` confinement and read-only mount refusals | `omnia:vcs/{store,workspace,transport}` | `omnia-git` | `conformance::vcs_flow` |

### Where the scenarios run

1. **`WasiProvider`** (`crates/test-programs/programs/conformance/portable.rs`, driven by `conformance_portable` in `crates/test/tests/host.rs`) — the scenarios inside a real guest, through omnia's runtime, over every in-memory default. This is the run that holds the runtime to the contract.
2. **`omnia_test::guest::Provider`** (`crates/test/tests/conformance.rs`) — the stateful scenarios over the handler-rung doubles, each test also asserting what reached its double, so a handler tested natively is tested against the contract the runtime keeps.

A production backend can join through its crate's own suite: `omnia_test::host::Backends` takes the backend in place of a default, so the same guest runs over it behind a service-up gate.

---

## Composing providers in a guest

Domain handler crates depend on `omnia-sdk` alone and expose transport routers generic over the provider `P`. Only the guest root crate (`src/lib.rs`) chooses the provider.

### 1. Standard WASI component (`WasiProvider`)

In the guest root crate (`src/lib.rs`), wire the HTTP and messaging routers to `omnia_sdk::WasiProvider` with each transport's `export!`:

```rust,ignore
use omnia_sdk::api::{Router, http, messaging};

pub fn http_router<P: Capabilities>(provider: P) -> Router<P> { /* .. */ }
pub async fn messaging_router<P: Capabilities>(provider: P) -> Router<P> { /* .. */ }

http::export!(http_router);
messaging::export!(messaging_router);
```

Each invocation expands to the `#[omnia_wasi_otel::instrument]`-scoped WASI export on `all(target_arch = "wasm32", target_os = "wasi")` and to nothing on native targets, so `cargo nextest` compiles the guest library natively against `omnia_test::guest::Provider` without pulling `wasmtime` or `omnia-wasi-*` into the native dependency graph.

### 2. Overriding capabilities with `#[derive(omnia_sdk::Provider)]`

A guest that overrides one or more capabilities — for example, wrapping outbound HTTP in `omnia_http_cache::HttpCache` — derives `Provider`, delegates the remaining capabilities to `WasiProvider`, and passes the provider as the second argument to `export!`:

```rust,ignore
#[derive(Clone, Default, omnia_sdk::Provider)]
struct Cached {
    #[provider(HttpRequest)]
    http: HttpCache<WasiProvider, WasiProvider>,
    #[provider(Config, Identity, Publish, Broadcast, StateStore, BlobStore, DocumentStore, TableStore)]
    wasi: WasiProvider,
}

http::export!(http_router, Cached::default());
```

---

## Internal naming conventions

Each symbol prefix in the workspace names one layer:

- **`Wasi*` (`WasiKeyValue`, `WasiBlobStore`, `WasiSql`, ...)** — `wasmtime` linker hosts in `omnia-wasi-*` and `omnia-plugin`. Never referenced by guest handler code.
- **`Wasi*View<'a>`** — borrowed `(client, table)` store views generated by `omnia_core::wasi_view!`.
- **`WasiProvider`** — the single zero-sized guest provider in `omnia-sdk` implementing every capability trait on `wasm32`.
- **`*Client` (`KeyValueClient`, `SqlClient`, ...)** — engine-free service contracts in `omnia-backend`, one per host that dispatches to a backend.
- **`*Default` (`KeyValueDefault`, `SqlDefault`, ...)** — in-memory or process default implementations in `omnia-wasi-*`.
- **Wire constants (`omnia_backend::keyvalue::BUCKET`, `omnia_backend::vault::LOCKER`)** — defined once in `omnia-backend`, re-exported by `omnia-sdk`, and shared by `WasiProvider` and the test harness so keys and secrets land in the same bucket and locker.
- **SDK capability trait names (`StateStore`, `DocumentStore`, `TableStore`, `HttpRequest`, `Publish`, ...)** — kept distinct from domain nouns so handler UFCS never collides with `axum::extract::State`, `docstore::Document`, `sea_query::Table`, or the `http` crate.
