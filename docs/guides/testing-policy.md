# Testing Policy

How the Omnia repository tests *itself*. The binding rules are also in the repository `AGENTS.md` (Testing policy); this page is the practical walk-through.

## The tiers

- **End-to-end tests** are the primary tier for `wasi-*` host crates: a real guest component driven through omnia's own runtime against an inline scenario backend, pinning the whole boundary (guest bindings → linker → host binding → backend and back). The exemplar is `crates/wasi-model/tests/model.rs`.
- **Unit tests** cover deterministic logic wherever it lives: parsers, codecs, filter/type translation, route matching, macro token expansion, guest-side library code. If a behavior is a pure function no guest boundary reaches (e.g. `Format::parse` candidate extraction, which backends drive directly), it is a unit test next to that logic.
- **Live tests** (in the `omnia-backends` repo) are the acceptance tier for production backends: `#[ignore]`-gated, credential-gated, driving the backend's `WasiXxxCtx` against the real service.

Guest-instantiating tests exist **only** through the shared pipeline below. Do not compile, deserialize, or instantiate a WASM guest ad hoc inside an individual test.

## The e2e pipeline

Two unpublished crates, patterned on wasmtime's `test-programs`:

- **`crates/test-programs`** holds the guest scenario programs, one `[[example]]` cdylib per scenario (`programs/<capability>/<scenario>.rs`). The example, path constant, and host test identity is `<capability>_<scenario>` (`model_echo_text`). `test-utils`'s build script generates the `[[example]]` stanzas from that tree. Each program asserts what the guest observes across the boundary and traps on failure; shared helpers live in its `src/lib.rs`. Everything is `#![cfg(target_arch = "wasm32")]`; the native build is empty.
- **`crates/test-utils`** compiles every program to a `wasm32-wasip2` component from its `build.rs` (into `OUT_DIR`, so plain `cargo make test` is self-contained), and generates one `pub const <NAME>: &str` artifact path per program plus a `foreach_<capability>!` macro. It also exports the capability-agnostic harness: `run_host::<H, B>` builds a one-shot `wasi:cli` command deployment from a wasm path, mounts, and a backend bundle, linking the single host under test (`run_command` takes a `link` closure for multi-host suites), and `scratch` mints a per-test workspace directory removed on drop.

A host crate's suite is one flat file per interface in its root `tests/` directory (`crates/wasi-model/tests/model.rs`). The file:

- invokes `test_utils::foreach_<capability>!();` so a guest program without a matching, identically named test fails to compile;
- defines its scenario backends inline next to the tests (see below);
- runs each guest with `test_utils::run_host` (via a small local `run_guest` wrapper supplying the `Has<Capability>` bundle and requiring `ExitStatus::SUCCESS`), then asserts any host-side effects (recorded requests, filesystem contents).

Assertions split by vantage point: the guest asserts what crosses the boundary to it (a panic traps and fails the host test); the host test asserts wire fidelity and side effects.

## Canned model backends

There is no shared model double: each test defines the backend it needs, inline, next to the test. The in-tree echo `ModelDefault` covers scenarios where the answer does not matter, or where its schema rejection is itself under test. A canned `WasiModelCtx` answering every completion with one fixed value is all the happy path needs — no network, no credentials, fully deterministic, and (unlike an echo) able to satisfy `format::schema`:

```rust,noplayground
use std::sync::Arc;

use futures::FutureExt as _;
use omnia_wasi_model::{Answer, FutureResult, Request, ToolHost, WasiModelCtx};
use serde_json::Value;

#[derive(Clone, Debug)]
struct Canned(Value);

impl WasiModelCtx for Canned {
    fn complete(&self, _request: Request, _tools: Arc<dyn ToolHost>) -> FutureResult<Answer> {
        let value = self.0.clone();
        async move { Ok(value.into()) }.boxed()
    }
}
```

## Running

```bash
cargo make test                                   # `cargo nextest run --locked --all --all-features`
cargo test --doc --all-features --workspace       # doc tests
```

`cargo-nextest` must be installed with `--locked` (`cargo install --locked cargo-nextest`). The `wasm32-wasip2` target must be installed (`rust-toolchain.toml` pins it); `test-utils`'s build script needs it to compile the guest programs.

## Naming

A test name is the scenario (`set_then_get`), not a restated expectation (`set_then_get_round_trips`).
