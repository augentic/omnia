# Testing Your Guests

Omnia's testing approach is integration-first: the boundary that matters is the guest–host seam, so tests load a real `.wasm`, link real hosts, and drive requests through the actual WIT boundary — no mocks between your guest and the runtime. This guide shows how to test *your* guests with the `omnia-testkit` scaffolding.

If you are contributing to Omnia itself, the repository's own seam suite and coverage rules are in [Seam Suite and Testing Policy](testing-policy.md).

## A first guest test in five minutes

1. Build your guest (tests never invoke Cargo themselves):

```bash
cargo build --example myguest-wasm --target wasm32-wasip2
```

2. Add the testkit to your dev-dependencies:

```toml
[dev-dependencies]
omnia-testkit.workspace = true
```

3. Assemble a single-guest runtime over your backend bundle and drive it in-process:

```rust,noplayground
let runtime = single_guest("myguest_wasm.wasm", bundle)
    .await?
    .host::<WasiHttp>()?
    .host::<WasiKeyValue>()?
    .into_runtime()?;

let response = http::post(&runtime, "/items", body).await?;
assert!(response.status().is_success());
```

`http` drives the guest's `wasi:http/handler` export directly — no TCP socket, no port collisions, and the whole request still crosses the real WIT boundary.

## The test taxonomy

| Kind           | What it covers                                                                                                   | How it runs                                             |
| -------------- | ---------------------------------------------------------------------------------------------------------------- | ------------------------------------------------------- |
| **Pure tier**  | Deterministic, service-free logic: parsers, codecs, filter/type translation, macro expansion, guest-native logic | Ordinary unit tests (Nextest, process-per-test)         |
| **Seam tier**  | Guests driven through the real runtime against the default (in-memory) backends                                  | Integration tests using `omnia-testkit`                 |
| **Live tests** | A production backend's `WasiXxxCtx` against the real service (`#[ignore]`-gated, in the `omnia-backends` repo)         | Local only                                              |

Anything that crosses a WASI interface belongs at the seam, not in a unit test with mocks.

## The testkit

`omnia-testkit` is a dev-only crate. Helpers:

- **`find_guest("name_wasm.wasm")`** — locates the built guest artifact (serialized `.bin` preferred, loaded via deserialization instead of JIT compilation; else the `.wasm`), panicking with build instructions when missing. No lazy builds, no silent skips.
- **`single_guest(file, bundle)`** — assembles a single-guest deployment over a backend bundle: `single_guest("x_wasm.wasm", bundle).await?.host::<WasiHttp>()?...into_runtime()?`.
- **`temp_manifest(toml)`** — writes a deployment manifest to a unique temp file, removed on drop, for tests that need multi-guest deployments, routes, or mounts.
- **`http`** — drives a guest's `wasi:http/handler` export in-process, with no TCP socket, e.g. `http::post(&runtime, "/", body)`.
- **`model`** — model doubles serving both faces of the `wasi-model` boundary.

### Testing model-consuming logic

`model::Scripted` returns FIFO successes or typed errors:

```rust,noplayground
use omnia_guest::model::{Model, Request};
use omnia_testkit::model::Scripted;

let model = Scripted::answers(["first", "second"]);
let first = model.create(Request::default()).await?;
assert_eq!(first.answer, "first");
```

Call `Scripted::assert_exhausted` at the end of a test when every scripted turn must be consumed. An unexpected extra call returns a deterministic `Error::Backend`; it does not panic.

`Scripted` also implements the host-side `WasiModelCtx`, so the same double serves integration tests and example runtimes: script host answers with `Scripted::json` (one JSON value) or `Scripted::values` (ordered `Answer` rows) and install the clone as the deployment's model backend. The double never runs tools; a request with no scripted result remaining fails with `model script exhausted`.

## Anatomy of a seam test

The pattern has three parts:

**1. A backend bundle with accessor impls** (mirroring the `runtime!` macro's generated `Backends`), keeping clones of the shared in-memory backends as probes:

```rust,noplayground
#[derive(Clone)]
pub struct Bundle {
    http: HttpDefault,
    keyvalue: KeyValueDefault,
    // ... every interface the guest imports
}

impl HasKeyValue for Bundle {
    fn keyvalue_ctx(&mut self) -> &mut dyn WasiKeyValueCtx {
        &mut self.keyvalue
    }
}
```

**2. Build the runtime** with `single_guest` as above (build it once and share it when many tests drive the same guest).

**3. Drive the guest and assert both sides of the seam** — the guest's response *and* the effect that landed in the host backend:

```rust,noplayground
#[test]
fn set_then_get() -> Result<()> {
    RT.block_on(async {
        let response = http::post(&runtime, "/keyvalue?key=k1", "payload").await?;
        assert!(response.status().is_success(), "guest completes the keyvalue round-trip");

        // The guest stored the body under `k1`; the shared backend must now
        // hold that write.
        let bucket = bundle.keyvalue.open_bucket("omnia_bucket".to_owned()).await?;
        let stored = bucket.get("k1".to_owned()).await?;
        assert_eq!(stored.as_deref(), Some(b"payload".as_slice()), "the write reached the host");

        Ok(())
    })
}
```

That second assertion is the point: a `200` proves the call crossed the WIT boundary without trapping; reading the shared backend (the **probe**) proves the write actually happened host-side rather than being swallowed.

When several tests share one runtime and its backends, derive keys/ids from a per-test unique suffix so concurrent tests never collide.

## Multi-guest and manifest-driven tests

For deployments with routes, mounts, or linking, either build a [`Manifest`](../../crates/omnia/src/deployment/manifest.rs) programmatically or generate one with `temp_manifest` and load it:

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
    .manifest(Manifest::from_config(manifest.path())?)
    .build::<StoreCtx<Bundle>>()
    .await?;
```

## Testing against real services

Production backends are tested in the [`omnia-backends`](https://github.com/augentic/omnia-backends) repo with `#[ignore]`-gated live tests that drive `WasiXxxCtx` against the actual service:

```bash
docker compose -f docker/redis.yaml up -d       # from the omnia repo's docker/ files
cargo nextest run -p omnia-redis --run-ignored all
```

See [Production Backends](production-backends.md#verifying-against-the-real-service) for per-backend requirements.

## Naming

A test name is the scenario (`set_then_get`), not a restated expectation (`set_then_get_round_trips`).
