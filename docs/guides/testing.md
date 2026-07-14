# Testing Guests and Runtimes

Omnia's testing approach is integration-first: the boundary that matters is the guest–host seam, so tests load a real `.wasm`, link real hosts, and drive requests through the actual WIT boundary. This guide shows how the two test tiers fit together and how to write seam tests using the `omnia-testkit` scaffolding.

The rationale and rules are codified in the repository `AGENTS.md` (Testing policy); this is the practical walk-through.

## The test taxonomy

| Kind           | What it covers                                                                                                   | How it runs                                             |
| -------------- | ---------------------------------------------------------------------------------------------------------------- | ------------------------------------------------------- |
| **Pure tier**  | Deterministic, service-free logic: parsers, codecs, filter/type translation, macro expansion, guest-native logic | `cargo make test` (Nextest, process-per-test, parallel) |
| **Seam tier**  | Guests driven through the real runtime against the default (in-memory) backends                                  | `cargo make test-seam` (one process, shared fixtures)   |
| **Live tests** | A production backend's `WasiXxxCtx` against the real service (`#[ignore]`-gated, in the `backends` repo)         | Local only                                              |

Anything that crosses a WASI interface belongs at the seam, not in a unit test with mocks. Guest-side logic that can't be instrumented as `.wasm` (coverage tooling limitation) keeps native unit tests.

## The seam suite

All seam tests live in one unpublished package, `crates/seam-suite`, compiled into a single integration-test binary (`tests/seam/main.rs` plus one module per scenario). Running them in one process lets every scenario share:

- one tokio runtime (`fixture::RT`),
- one conformance runtime — component, linker, and `InstancePre` built once (`fixture::conformance()`),
- probe handles onto every shared in-memory backend, so tests assert host-side effects.

The conformance guest (`examples/conformance/guest.rs`) exposes one HTTP route per WASI interface and imports the real guest APIs. Scenarios that need their own deployment shape (CLI, model completion/workspace, HTTP routing, MCP, typed guest API, guest-to-guest linking) build their own runtime from their own guest but still share the suite process.

Tests sharing the conformance backends take their keys/ids from `fixture::unique(..)` so concurrent scenarios never collide.

## Guest artifacts are explicit

Tests never invoke Cargo. `find_guest` is locate-only and fail-fast: it looks for a serialized `.bin` (preferred, loaded via deserialization instead of JIT compilation) or a `.wasm` under the example target directory and panics with build instructions when neither exists.

Build (and serialize) exactly the guests the seam suite drives with:

```bash
cargo make test-guests
```

`cargo make test-seam` depends on that task, so the one-command path is just `test-seam`. The full example set (including guests without seam coverage) still builds with `cargo make examples` for main/scheduled validation.

## The testkit

`omnia-testkit` is a dev-only, unpublished crate. Helpers:

- **`find_guest("name_wasm.wasm")`** — locates the built guest artifact (serialized `.bin` preferred), panicking with build instructions when missing. No lazy builds, no silent skips.
- **`single_guest(file, bundle)`** — assembles a single-guest deployment over a backend bundle: `single_guest("x_wasm.wasm", bundle).await?.host::<WasiHttp>()?...into_runtime()?`.
- **`temp_manifest(toml)`** — writes a deployment manifest to a unique temp file, removed on drop, for tests that need multi-guest deployments, routes, or mounts.
- **`http`** — drives a guest's `wasi:http/handler` export in-process, with no TCP socket, e.g. `http::post(&runtime, "/", body)`.
- **`guests`** (binary) — precompiles built `.wasm` guests into `.bin` components via Omnia's compile path; invoked by `test-guests`.
- **`model`** — model doubles serving both faces of the `wasi-model` boundary.

### Testing model-consuming core logic

```toml
[dev-dependencies]
omnia-testkit.workspace = true
```

`model::Scripted` returns FIFO successes or typed errors:

```rust,noplayground
use omnia_guest::model::{Model, Request};
use omnia_testkit::model::Scripted;

let model = Scripted::answers(["first", "second"]);
let first = model.create(Request::default()).await?;
assert_eq!(first.answer, "first");
```

Call `Scripted::assert_exhausted` at the end of a test when every scripted turn must be consumed. An unexpected extra call returns a deterministic `Error::Backend`; it does not panic.

`Scripted` also implements the host-side `WasiModelCtx`, so the same double serves seam tests and example runtimes: script host answers with `Scripted::json` (one JSON value) or `Scripted::values` (ordered `Answer` rows) and install the clone as the deployment's model backend. The double never runs tools; a request with no scripted result remaining fails with `model script exhausted`.

## Anatomy of a seam test

The suite's shared fixture (`crates/seam-suite/tests/seam/fixture.rs`) is the exemplar. The pattern:

**1. A backend bundle with accessor impls** (mirroring the `runtime!` macro's generated `Backends`), keeping clones of the shared in-memory backends as probes:

```rust,noplayground
#[derive(Clone)]
pub struct Bundle {
    http: HttpDefault,
    otel: CapturingOtel,
    keyvalue: KeyValueDefault,
    // ... every interface the conformance guest imports
}

impl HasKeyValue for Bundle {
    fn keyvalue_ctx(&mut self) -> &mut dyn WasiKeyValueCtx {
        &mut self.keyvalue
    }
}
```

**2. Build the runtime once for the suite** and expose it through a lazily initialized fixture:

```rust,noplayground
let runtime = single_guest("conformance_wasm.wasm", bundle)
    .await?
    .host::<WasiHttp>()?
    .host::<WasiKeyValue>()?
    // ... remaining hosts
    .into_runtime()?;
```

**3. Drive the guest and assert both sides of the seam** — the guest's response *and* the effect that landed in the host backend:

```rust,noplayground
#[test]
fn set_then_get() -> Result<()> {
    fixture::RT.block_on(async {
        let fx = fixture::conformance().await?;
        let key = unique("kv-set");

        let response = http::post(&fx.runtime, &format!("/keyvalue?key={key}"), "payload").await?;
        assert!(response.status().is_success(), "guest completes the keyvalue round-trip");

        // The guest stored the body under `key`; the shared backend must now
        // hold that write.
        let bucket = fx.keyvalue.open_bucket("omnia_bucket".to_owned()).await?;
        let stored = bucket.get(key).await?;
        assert_eq!(stored.as_deref(), Some(b"payload".as_slice()), "the write reached the host");

        Ok(())
    })
}
```

That second assertion is the point: a `200` proves the call crossed the WIT boundary without trapping; reading the shared backend proves the write actually happened host-side rather than being swallowed.

## Multi-guest and manifest-driven tests

For deployments with routes, mounts, or linking, generate the manifest with `temp_manifest` and pass it to the builder (see the `routing` and `guest_link` scenarios in the suite):

```rust
let manifest = temp_manifest(r#"
    [[guest]]
    id = "api"
    source.path = "/abs/path/to/api_wasm.wasm"

    [[route.http]]
    prefix = "/"
    guest = "api"
"#)?;
let deployment = DeploymentBuilder::new()
    .config(manifest.path().to_path_buf())
    .build::<StoreCtx<Bundle>>()
    .await?;
```

## Running the tests

```bash
cargo make test        # pure tier: Nextest, excludes the seam suite
cargo make test-seam   # seam tier: builds + serializes guests, then one-process suite
cargo test --doc --all-features --workspace   # doc tests
```

`cargo-nextest` must be installed with `--locked` (`cargo install --locked cargo-nextest`). The Nextest default filter (`.config/nextest.toml`) excludes `omnia-seam-suite`, so `cargo nextest run --all` never accidentally runs seam tests process-per-test — and never silently skips a missing guest either: a seam run with missing artifacts fails with build instructions.

## Testing against real services

Production backends are tested in the [`backends`](https://github.com/augentic/backends) repo with `#[ignore]`-gated live tests that drive `WasiXxxCtx` against the actual service:

```bash
docker compose -f docker/redis.yaml up -d       # from the omnia repo's docker/ files
cargo nextest run -p omnia-redis --run-ignored all
```

See [Production Backends](production-backends.md#verifying-against-the-real-service) for per-backend requirements.

## Naming and hygiene

- A test name is the scenario (`set_then_get`), not a restated expectation (`set_then_get_round_trips`).
- When a seam test supersedes a unit-test module, delete the unit tests in the same change, with coverage evidence (`cargo llvm-cov`) that nothing regressed.
