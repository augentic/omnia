# Testing Your Guests

Omnia's testing approach is integration-first: the boundary that matters is the guest–host seam, so tests load a real `.wasm`, link real hosts, and drive requests through the actual WIT boundary — no mocks between your guest and the runtime. This guide shows how to test *your* guests with the `omnia-testkit` scaffolding.

If you are contributing to Omnia itself, the repository's own coverage rules are in [Testing Policy](testing-policy.md).

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
| **Seam tier**  | Guests driven through the real runtime against the default (in-memory) backends                                  | Integration tests using `omnia-testkit` (Nextest, process-per-test; guests pre-built) |
| **Live tests** | A production backend's `WasiXxxCtx` against the real service (`#[ignore]`-gated, in the `omnia-backends` repo)         | Local only                                              |

Test where the behavior lives: deterministic logic gets unit tests, and the seam gets a test when the boundary itself is the contract — never a unit test with mocks standing in for the runtime.

## The testkit

`omnia-testkit` is a dev-only crate. Helpers:

- **`find_guest("name_wasm.wasm")`** — locates the built guest artifact (serialized `.bin` preferred, loaded via deserialization instead of JIT compilation; else the `.wasm`), panicking with build instructions when missing. No lazy builds, no silent skips.
- **`single_guest(file, bundle)`** — assembles a single-guest deployment over a backend bundle: `single_guest("x_wasm.wasm", bundle).await?.host::<WasiHttp>()?...into_runtime()?`.
- **`temp_manifest(toml)`** — writes a deployment manifest to a unique temp file, removed on drop, for tests that need multi-guest deployments, routes, or mounts.
- **`http`** — drives a guest's `wasi:http/handler` export in-process, with no TCP socket, e.g. `http::post(&runtime, "/", body)`.

### Testing model guests

There is no shared model double: each test defines the backend it needs, inline, next to the test (see `crates/seam-suite/tests/model.rs` for the pattern — a canned happy-path backend alongside purpose-built probes like `PathProbe` and `WriteProbe`). The in-tree echo `ModelDefault` covers scenarios where the answer does not matter, or where its schema rejection is itself under test. A canned backend answering every completion with one fixed value is all the happy path needs:

```rust,noplayground
use std::sync::Arc;

use futures::FutureExt as _;
use omnia_wasi_model::{Answer, FutureResult, Request, ToolHost, WasiModelCtx};
use serde_json::Value;

#[derive(Clone, Debug)]
struct Canned(Value);

impl WasiModelCtx for Canned {
    fn complete(&self, _request: Request, _tools: Arc<dyn ToolHost>) -> FutureResult<Answer> {
        let answer = Answer { value: self.0.clone(), usage: None, transcript: None };
        async move { Ok(answer) }.boxed()
    }
}
```

Install it as the deployment's model backend and assert on the guest-visible output; unlike an echo, a canned JSON value can satisfy a guest's `format::schema` request.

## Anatomy of a seam test

The pattern has three parts:

**1. A backend bundle with accessor impls** (mirroring the `runtime!` macro's generated `Backends`):

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

**2. Build the runtime** with `single_guest` as above.

**3. Drive the guest and assert the guest-visible outcome:**

```rust,noplayground
#[tokio::test]
async fn set_then_get() -> Result<()> {
    let response = http::post(&runtime, "/keyvalue?key=k1", "payload").await?;
    assert!(response.status().is_success(), "guest completes the keyvalue round-trip");
    Ok(())
}
```

A success response proves the call crossed the WIT boundary and that the guest's own checks (it read back what it wrote) held. Reach for a **probe** — a clone of a shared backend, read host-side — only when the guest cannot observe the effect itself: a message that must land on the broker, a frame that must reach a connected peer, a write that must be denied. For example, subscribing on the broker before the guest publishes:

```rust,noplayground
let mut subscription = bundle.messaging.connect().await?.subscribe().await?;
let response = http::post_json(&runtime, "/publish", payload).await?;
assert!(response.status().is_success());

let message = timeout(Duration::from_secs(5), subscription.next())
    .await?
    .context("no delivery")?;
assert_eq!(message.payload, payload.as_bytes(), "the publish reached the broker");
```

A probe that re-reads the same in-memory store the guest just wrote and read back adds no information — assert one side of the seam, the one only that side can see.

When several tests share one runtime and its backends, derive keys/ids from a per-test unique suffix so concurrent tests never collide.

## Multi-guest and manifest-driven tests

For deployments with routes, mounts, or linking, either build a [`Manifest`](../../crates/omnia/src/deployment/manifest.rs) programmatically or generate one with `temp_manifest` and load it:

```rust
let manifest = temp_manifest(r#"
    [[guest]]
    id = "api"
    source.path = "/abs/path/to/api_wasm.wasm"
    routes.http = ["/"]
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
