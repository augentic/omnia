# Testing Guests and Runtimes

Omnia's testing approach is integration-first: the boundary that matters is the guest–host seam, so tests load a real `.wasm`, link real hosts, and drive requests through the actual WIT boundary. This guide shows how to write those tests for your own guests using the `omnia-testkit` scaffolding, and where unit tests still belong.

The rationale and rules are codified in the repository `AGENTS.md` (Testing policy); this is the practical walk-through.

## The test taxonomy

| Kind | What it covers | Where it runs |
| ---- | -------------- | ------------- |
| **Unit tests** | Pure, deterministic logic only: parsers, codecs, filter/type translation, macro expansion | CI |
| **Seam tests** | A guest driven through the real runtime against a default (in-memory) backend | CI |
| **Live tests** | A production backend's `WasiXxxCtx` against the real service (`#[ignore]`-gated, in the `backends` repo) | Local only |

Anything that crosses a WASI interface belongs at the seam, not in a unit test with mocks. Guest-side logic that can't be instrumented as `.wasm` (coverage tooling limitation) keeps native unit tests.

## The testkit

`omnia-testkit` is a dev-only, unpublished crate with three helpers:

- **`find_guest("name_wasm.wasm")`** — locates the built guest, building example guests on first use. It encodes a "fail in CI, skip locally" policy: locally a missing guest skips the test; under CI it fails, so the pipeline never passes vacuously.
- **`temp_manifest(toml)`** — writes a deployment manifest to a unique temp file, removed on drop, for tests that need multi-guest deployments, routes, or mounts.
- **`http`** — drives a guest's `wasi:http/handler` export in-process, with no TCP socket, e.g. `http::post(&runtime, "/", body)`.

Add it as a dev-dependency (path-only; it is not published):

```toml
[dev-dependencies]
omnia-testkit = { path = "../testkit" }
```

## Anatomy of a seam test

The `wasi:keyvalue` seam test is the exemplar. It assembles the same runtime the `runtime!` macro would generate — but by hand, so the test can keep a probe handle into the shared backend and assert host-side effects.

**1. A backend bundle with accessor impls** (mirroring the macro's generated `Backends`):

```27:50:crates/wasi-keyvalue/tests/seam.rs
#[derive(Clone)]
struct Bundle {
    http: HttpDefault,
    otel: OtelDefault,
    keyvalue: KeyValueDefault,
}

impl HasHttp for Bundle {
    fn http_view<'a>(&'a mut self, table: &'a mut ResourceTable) -> WasiHttpCtxView<'a> {
        self.http.as_view(table)
    }
}

impl HasOtel for Bundle {
    fn otel_ctx(&mut self) -> &mut dyn WasiOtelCtx {
        &mut self.otel
    }
}

impl HasKeyValue for Bundle {
    fn keyvalue_ctx(&mut self) -> &mut dyn WasiKeyValueCtx {
        &mut self.keyvalue
    }
}
```

**2. Build the runtime over the guest**, keeping a clone of the backend as a probe (the in-memory defaults share state across clones):

```55:80:crates/wasi-keyvalue/tests/seam.rs
async fn runtime() -> Result<Option<(Runtime<Bundle>, KeyValueDefault)>> {
    let Some(wasm) = find_guest("keyvalue_wasm.wasm") else {
        return Ok(None);
    };

    let bundle = Bundle {
        http: HttpDefault::connect().await.context("connecting http")?,
        otel: OtelDefault::connect().await.context("connecting otel")?,
        keyvalue: KeyValueDefault::connect().await.context("connecting keyvalue")?,
    };
    let store_probe = bundle.keyvalue.clone();

    let mut deployment =
        DeploymentBuilder::new().wasm(wasm).build::<StoreCtx<Bundle>>().await.context("build")?;
    deployment.host::<WasiHttp, Bundle>().context("link http")?;
    deployment.host::<WasiOtel, Bundle>().context("link otel")?;
    deployment.host::<WasiKeyValue, Bundle>().context("link keyvalue")?;
    let registry = deployment.into_registry().context("assemble registry")?;

    let runtime = Runtime::from_parts(
        Arc::new(registry),
        Vec::new(),
        Arc::new(MountRegistry::default()),
        bundle,
    );
    Ok(Some((runtime, store_probe)))
}
```

**3. Drive the guest and assert both sides of the seam** — the guest's response *and* the effect that landed in the host backend:

```83:99:crates/wasi-keyvalue/tests/seam.rs
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn set_then_get() -> Result<()> {
    let Some((runtime, store)) = runtime().await? else {
        return Ok(());
    };

    let response = http::post(&runtime, "/", "payload-value").await?;
    assert!(response.status().is_success(), "guest completes the keyvalue round-trip");

    // The guest stored the request body under `my_key` in `omnia_bucket`; the
    // shared backend must now hold that write.
    let bucket = store.open_bucket("omnia_bucket".to_owned()).await.context("open bucket")?;
    let stored = bucket.get("my_key".to_owned()).await.context("read my_key")?;
    assert_eq!(stored.as_deref(), Some(b"payload-value".as_slice()), "the write reached the host");

    Ok(())
}
```

That second assertion is the point: a `200` proves the call crossed the WIT boundary without trapping; reading the shared backend proves the write actually happened host-side rather than being swallowed.

## Multi-guest and manifest-driven tests

For deployments with routes, mounts, or linking, generate the manifest with `temp_manifest` and pass it to the builder:

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
cargo nextest run --all --all-features --no-tests=pass   # workspace tests, incl. seam tests
cargo test --doc --all-features --workspace              # doc tests
```

`cargo-nextest` must be installed with `--locked` (`cargo install --locked cargo-nextest`). Seam tests build their guest on first run, so the first invocation is slower.

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
