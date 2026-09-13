# Testing Omnia-Based Code

How to test a guest crate or an embedder built on omnia, using the `omnia-test` crate. The [Testing Policy](testing-policy.md) covers how the omnia repository tests itself; this page is for code that depends on omnia.

## The three rungs

A guest is a handler compiled to a `wasm32-wasip2` component and run by a host runtime. Each of those layers is a place to test, and `omnia-test` puts one feature behind each:

| Rung | Feature | What runs | Against |
| ---- | ------- | --------- | ------- |
| Handler | `guest` | The handler's own logic, compiled natively | One in-memory double per `omnia_guest` capability trait |
| Component | `host` | The compiled component, through omnia's own runtime | `Backends`, the twelve in-memory host defaults with the model scripted |
| Fixture | `build` | The nested cargo build a `build.rs` runs to produce those components | `std` only |

There is no default feature: a consumer names the rung it uses. Most guest crates need only the handler rung; the component rung is for behaviour that exists at the WASI boundary (the wiring a `runtime!` compiles in, plugin loading, workspace tools); the fixture rung is for a crate that drives its own components through the component rung.

## Depending on the crate

Two lines cover a typical consumer:

```toml
[target.'cfg(not(target_arch = "wasm32"))'.dev-dependencies]
omnia-test = { version = "0.36", features = ["guest", "host"] }

[build-dependencies]
omnia-test = { version = "0.36", features = ["build"] }
```

The target gate on the dev line is the canonical shape for a guest crate: handler tests compile natively, and the component never sees the crate. The crate is also empty on `wasm32` (`#![cfg(not(target_arch = "wasm32"))]`), so an ungated line resolves on both targets and contributes nothing to the component; the gate simply keeps the host crates out of the `wasm32` dependency graph. The `build` line stays `std`-only by contract (omnia's CI guards its dependency tree), so it never pulls the runtime into a `build.rs`.

## Handler rung: `omnia_test::guest::Provider`

A guest declares its production provider as one struct with an empty impl per capability it needs; on `wasm32` each impl picks up the WASI-backed default bodies.

```rust,ignore
/// The connector's capabilities, on the WASI defaults.
struct Provider;

impl Config for Provider {}
impl Publish for Provider {}
```

A test runs the same handler against `omnia_test::guest::Provider`, which implements every capability trait over one `pub` field per double, each seeded through a consuming builder of the same name:

```rust,ignore
use omnia_test::guest::{MapConfig, Provider};

fn provider() -> Provider {
    Provider::default().config(MapConfig::default().with([("ENV", "dev")]))
}

#[tokio::test]
async fn device_site_header() {
    let provider = provider();
    handle(&provider, payload).await;

    let published = provider.publish.sent();
    assert_eq!(published[0].0, "dev-realtime-tally-apc.v2");
}
```

The handler's own bounds pick out the capabilities it touches, so a test seeds only those fields and leaves the rest at their defaults. `StateStore` and `BlobStore` share one `storage` field — one `Memory` serves both, the shape a production provider's single storage backend has. The full field/double table is on the struct's rustdoc: `MapConfig`, `MatchedHttp`, `FixedIdentity`, `Sink` (for `Publish` and `Broadcast`), `Memory`, `MemoryDocs`, `ScriptedTables`, `Scripted` (for `Model`), `ScriptedLoader` (for `Plugins`).

Every capability trait is also implemented for `Arc<T>`, `&T`, and `Box<T>`, so a handler bounded on `P: StateStore` accepts `Arc<Memory>` directly.

That covers the two other shapes a test provider takes. A handler bounded on a single capability needs no struct at all: pass `Arc<Memory>` (or `&Memory`) where the provider goes. A bespoke struct — generic over a storage type, say, or holding a double behind an `Arc` — hand-writes its impls, each method forwarding to the field that carries the double, exactly as a production provider over a custom backend would.

## Component rung: `Deployment` over `Backends`

`Deployment` describes one command-mode deployment — guests, mounts, arguments, link interfaces, and the directory the `.` path location serves — and runs it over a backend bundle. Built from nothing it drives a single component through the host you name:

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

The guest asserts what crosses the boundary to it (a panic traps and fails the host test); the host side asserts wire fidelity and side effects through the handles the bundle keeps — `backends.model.seen()`, `backends.state(key)`, `backends.object(container, name)`, the scratch directory.

### The overlay on a `runtime!`

An embedder's `runtime!` module generates, beside `main` and `run`, a `manifest()` accessor for the deployment it compiled in and a `Hooks` type carrying its wiring. `Deployment::from(runtime::manifest())` overlays that base — adding guests or mounts, re-marking the command guest, rewriting the `.` path location with `path_root` — and `run_with::<runtime::Hooks, _>` drives it through the production wiring over a test bundle:

```rust,ignore
let status = Deployment::from(runtime::manifest())
    .run_with::<runtime::Hooks, _>(Backends::defaults().await.model(model))
    .await?;
```

The generated `main` and `run` stay untouched; the test reaches the same hosts and the same plugin acquisition policy the binary would. This is the rung for "does the wiring I compiled in actually link and serve this guest".

### `Backends`

`Backends::defaults()` is deterministic: it reads no environment variable and opens no socket. Config answers from the map seeded by `Backends::config([("KEY", "value")])` alone, the websocket backend serves no listener, and the HTTP client and `SQLite` connection (one private `:memory:` database per bundle) are built from fixed options. `Backends::model(m)` swaps in any `WasiModelCtx`; `ScriptedModel` is the scripted one, sharing its `Script` core with the handler rung's `Scripted` so a script reads identically at both rungs. Each scripted completion may run tool calls and workspace `read`/`write`/`list` steps ahead of its answer, recording every exchange; an overrun answers the guest softly and fails the test at `assert_exhausted()`.

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

`Components` runs the nested `wasm32-wasip2` build into `OUT_DIR` (so plain `cargo test` is self-contained), and `write_gen` emits one `pub const <GROUP>_<SCENARIO>: &str` artifact path per program plus a `foreach_<group>!` macro that fails to compile unless an identically named test exists at the invocation site. `scan` discovers programs from a `<group>/<scenario>.rs` tree and `sync_examples` keeps the `[[example]]` stanzas in step; `examples([..])` names them explicitly instead. A repository whose components under test are shipped `cdylib` packages adds them to the same build with `scan_packages(dir).group(name)` (or `packages([..])`), so one `gen.rs` carries the components and the programs that drive them.

## Examples to read

- Handler rung: [`crates/tally-connector/tests/static.rs`](https://github.com/augentic/omnia-exemplar/blob/main/crates/tally-connector/tests/static.rs) in the exemplar — a `Config + Publish` handler run against `omnia_test::guest::Provider`, seeded config, published records asserted.
- Component rung and the overlay: [`crates/omnia-test/tests/host.rs`](../../crates/omnia-test/tests/host.rs) — `runtime_overlay` and `path_root` drive a `runtime!` module's `Hooks` through `Deployment::from(manifest())`.
- Fixture rung: [`crates/test-programs/build.rs`](../../crates/test-programs/build.rs), the build that feeds omnia's own e2e suites — in the guest package itself, which the `wasm32` no-op makes safe.
