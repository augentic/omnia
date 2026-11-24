# Testing Omnia-Based Code

How to test a guest crate or an embedder built on omnia, using the `omnia-test` crate. The [Testing Policy](testing-policy.md) covers how the omnia repository tests itself; this page is for code that depends on omnia.

## The rungs

A guest is a handler compiled to a `wasm32-wasip3` component and run by a host runtime. Its host calls are component imports, which panic off `wasm32`, so a handler is tested as the component it ships in. `omnia-test` puts one feature behind each place to test:

| Rung | Feature | What runs | Against |
| ---- | ------- | --------- | ------- |
| Component | `host` | The compiled component, through omnia's own runtime | `Backends`, the in-memory host defaults, each swappable for a scripted or recording backend |
| Contract | `contract` | The behaviour rows every backend of an interface meets, inside a guest | The host the guest runs over, and its backend |
| Fixture | `build` | The nested cargo build a `build.rs` runs to produce those components | `std` only |

There is no default feature: a consumer names the rung it uses. A guest crate takes the component and fixture rungs: its `build.rs` compiles its component, and its tests boot it. A backend crate adds the contract rows. A handler that reaches no host interface also runs natively, through `Router::handle`, `Router::run`, or the `axum::Router` a `Router` converts into.

## Depending on the crate

Two lines cover a typical consumer:

```toml
[target.'cfg(not(target_arch = "wasm32"))'.dev-dependencies]
omnia-test = { version = "0.39", features = ["host"] }

[build-dependencies]
omnia-test = { version = "0.39", features = ["build"] }
```

The target gate on the dev line keeps the runtime out of the component's build. Without `contract`, the crate is also empty on `wasm32`, so an ungated line resolves on both targets and contributes nothing to the component. The `build` line stays `std`-only by contract (omnia's CI guards its dependency tree), so it never pulls the runtime into a `build.rs`.

## Handler tests: the component over scripted backends

A handler test boots the guest's own component over `Backends`, swaps in a scripted or recording backend for each host the handler reaches, and drives one trigger in process:

- `Deployment::boot(backends, link)` links the hosts the guest imports and answers the booted `Runtime`.
- `send(&runtime, request)` serves one HTTP request and answers the reply with its body collected.
- `deliver(&runtime, topic, payload)` delivers one message and answers the `HandleError` the guest settles it with.
- `precompiled(path)` compiles the component ahead of time once, so a suite that boots per test pays for one compile.

```rust,ignore
use omnia_test::host::{Backends, Deployment, MatchedHttp, Sink, precompiled, send};
use omnia_wasi_http::WasiHttp;
use omnia_wasi_messaging::WasiMessaging;
use omnia_wasi_otel::WasiOtel;

#[tokio::test]
async fn rename() {
    let price = Response::new(Bytes::from_static(br#"{"price":12}"#));
    let http = MatchedHttp::default().on(Method::GET, "https://prices.test/items/7", price);
    let sink = Sink::default();
    let backends = Backends::defaults().await.http(http.clone()).messaging(sink.clone());
    let runtime = Deployment::new()
        .guest("items", precompiled(ITEMS_WASM).expect("the component precompiles"))
        .boot(backends, |deployment| {
            deployment.host::<WasiHttp>()?;
            deployment.host::<WasiMessaging>()?;
            deployment.host::<WasiOtel>()?;
            Ok(())
        })
        .await
        .expect("runtime boots");

    let response = send(&runtime, rename(7)).await.expect("rename served");
    assert_eq!(response.status(), StatusCode::CREATED);
    assert_eq!(http.requests().len(), 1);
    assert_eq!(sink.sent()[0].topic, "items.renamed");
    runtime.shutdown();
}
```

The `export!` macros open the guest's telemetry scope, which imports `omnia:otel`, so a deployment links `WasiOtel` beside the hosts its handlers reach. The scripted and recording backends each serve one host:

- `MatchedHttp` answers outbound `wasi:http` by method and URL, and records every request. It holds a guest to its `hosts` grant as `HttpDefault` does: a request outside the list fails with `http-request-denied` and is not recorded.
- `ScriptedTables` answers `omnia:sql` statements by predicate, and records each statement and transaction.
- `Sink` records every message and WebSocket send.
- `FixedIdentity` answers a fixed token for every identity, and records the scopes asked for.
- `ScriptedModel` answers completions from a script.

A call no rule matches fails in the guest with an error naming it, so the reply carries it to the assertion. `Backends::config([("KEY", "value")])` seeds configuration, and the in-memory defaults serve key-value, blob, document, and vault state. `backends.state(key)` and `backends.object(container, name)` read back what a guest wrote.

## Component rung: `Deployment` over `Backends`

`Deployment` describes one command-mode deployment — guests (`guest(name, path)` reads the component into bytes so it loads at boot, as an embedded guest does; `entry(GuestEntry::new(name, path))` declares one that loads at its first use), mounts (the read-only ones also the roots a guest's path load reads through), arguments, and the store and registries its package guests are acquired through (`store(root)` points the store at a scratch directory, where a release copied under its reference's file name loads with nothing fetched) — and runs it over a backend bundle. `captured()` wraps it so a run answers what the guests wrote to stdout and stderr beside the exit status. Built from nothing it drives a single component through the host you name:

```rust,ignore
use omnia::ExitStatus;
use omnia_test::host::{Backends, Deployment, ScriptedModel};
use omnia_wasi_model::WasiModel;

let backends = Backends::defaults().await.model(ScriptedModel::answering(["second"]));
let status = Deployment::new()
    .guest("echo", ECHO_WASM)
    .run_host::<WasiModel, _>(backends.clone())
    .await?;
assert_eq!(status, ExitStatus::SUCCESS);
assert_eq!(backends.model.seen()[0].messages, ["hi", "second"]);
backends.model.assert_exhausted();
```

The guest asserts what crosses the boundary to it (a panic traps and fails the host test); the host side asserts wire fidelity and side effects through the handles the bundle keeps — `backends.model.seen()`, `backends.state(key)` (the raw bytes in the `default` bucket), `backends.object(container, name)`, the scratch directory.

### The overlay on a `runtime!`

An embedder's `runtime!` module generates, beside `main` and `run`, a `manifest()` accessor for the deployment it compiled in and a `Hooks` type carrying its wiring. `Deployment::from(runtime::manifest())` overlays that base — adding guests, re-marking the command guest, replacing the binary's `.` mount with a scratch directory (`mount(scratch.mount(false))`; mounts dedup by name, last wins, before any directory opens, so a production root that does not exist under test is never touched) — and `run_with::<runtime::Hooks, _>` drives it through the production wiring over a test bundle:

```rust,ignore
let status = Deployment::from(runtime::manifest())
    .run_with::<runtime::Hooks, _>(Backends::defaults().await.model(model))
    .await?;
```

The generated `main` and `run` stay untouched; the test reaches the same hosts the binary would, and the same guest loader over the base manifest's guests plus any the overlay declares (`Deployment::entry`). This is the rung for "does the wiring I compiled in actually link and serve this guest".

### `Backends`

`Backends::defaults()` is deterministic: it reads no environment variable and opens no socket. Config answers from the map seeded by `Backends::config([("KEY", "value")])` alone, the websocket backend serves no listener, and the HTTP client and `SQLite` connection (one private `:memory:` database per bundle) are built from fixed options. The key-value backend declares the `default` bucket alone; a guest that names others runs over `keyvalue(KeyValueDefault::connect_with(ConnectOptions { buckets }).await?)`. The document-store, SQL, identity, vault, and echo model backends likewise serve `default` alone, and a guest that names others runs over `docstore(DocStoreDefault::connect_with(ConnectOptions { collections }).await?)`, `sql(SqlDefault::connect_with(ConnectOptions { database, databases }).await?)`, `identity(IdentityStub::connect_with(StubOptions { names }).await?)`, `vault(VaultDefault::connect_with(ConnectOptions { lockers }).await?)`, or `model(ModelDefault::connect_with(ConnectOptions { models }).await?)`. Every swappable host is a type parameter of `Backends` defaulting to its in-memory backend, with a consuming setter of the same name: `Backends::model(m)` swaps in any `ModelClient`, and `keyvalue`, `blobstore`, `docstore`, `sql`, `vault`, `messaging`, `identity`, `otel`, `vcs`, `http`, and `websocket` take any backend of theirs — a production backend crate's client, a scripted backend, or a recording wrapper around the default — so the same guests run over it. `ScriptedModel` is the scripted model, over the `Script` core. Each scripted completion may run tool calls and workspace `read`/`write`/`list` steps ahead of its answer, recording every exchange; an overrun answers the guest softly and fails the test at `assert_exhausted()`.

## Contract rows

`omnia_test::contract` holds one `async fn` per invariant every backend of `omnia:keyvalue`, `omnia:blobstore`, `omnia:docstore`, or `omnia:sql` meets. The rows run inside a guest, over the `omnia_sdk` bindings, so the `contract` feature builds on `wasm32`. A row panics when the backend behind the host breaks its invariant, which traps the guest and fails the test that booted it.

omnia's own guests run the rows over the in-memory defaults: [`programs/keyvalue/state_store.rs`](../../crates/test-programs/programs/keyvalue/state_store.rs), [`programs/blobstore/blob_store.rs`](../../crates/test-programs/programs/blobstore/blob_store.rs), [`programs/docstore/document_db.rs`](../../crates/test-programs/programs/docstore/document_db.rs), and [`programs/sql/sql_db.rs`](../../crates/test-programs/programs/sql/sql_db.rs). A production backend crate runs the same programs over its own client. Passing the rows is the first of the [provider obligations](../reference/provider-contract.md#obligations).

## Fixture rung: `Components` in `build.rs`

A crate that drives its own components through the component rung compiles them in its `build.rs`:

```rust,ignore
fn main() {
    omnia_test::build::Components::in_workspace("../..")
        .package("test-programs")
        .scan("crates/test-programs/programs")
        .sync_examples("crates/test-programs/Cargo.toml")
        .build()
        .write_gen("gen.rs");
}
```

`Components` runs the nested `wasm32-wasip3` build into `target/wasm32-fixtures` — a sibling of the outer profile directory shared by every outer configuration, so plain `cargo test` is self-contained and a version or toolchain bump rebuilds the tree incrementally instead of leaving the last one behind under a per-hash `OUT_DIR` — and `write_gen` emits one `pub const <GROUP>_<SCENARIO>: &str` artifact path per program plus a `foreach_<group>!` macro that fails to compile unless an identically named test exists at the invocation site. `scan` discovers programs from a `<group>/<scenario>.rs` tree and `sync_examples` keeps the `[[example]]` stanzas in step; `examples([..])` names them explicitly instead. A repository whose components under test are shipped `cdylib` packages adds them to the same build with `scan_packages(dir).group(name)` (or `packages([..])`), so one `gen.rs` carries the components and the programs that drive them.

## Examples to read

- Handler tests: [`crates/test/tests/host.rs`](../../crates/test/tests/host.rs) — `harness_flow` boots one guest over `ScriptedTables`, `MatchedHttp`, `Sink`, and `FixedIdentity`, and drives it with `send` and `deliver`.
- Component rung and the overlay: the same suite — `runtime_overlay` and `overlay_mount` drive a `runtime!` module's `Hooks` through `Deployment::from(manifest())`.
- Fixture rung: [`crates/test-programs/build.rs`](../../crates/test-programs/build.rs), the build that feeds omnia's own e2e suites — in the guest package itself, which the `wasm32` no-op makes safe.
