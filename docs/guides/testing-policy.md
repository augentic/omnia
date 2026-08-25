# Testing Policy

How the Omnia repository tests *itself*. The binding rules are also in the repository `AGENTS.md` (Testing policy); this page is the practical walk-through.

## The tiers

- **Unit tests** cover deterministic logic wherever it lives: parsers, codecs, filter/type translation, route matching, macro token expansion, guest-side library code, and backend semantics driven directly against a `WasiXxxCtx` trait. If a behavior can be pinned without instantiating a guest, it is a unit test.
- **Live tests** (in the `omnia-backends` repo) are the acceptance tier for production backends: `#[ignore]`-gated, credential-gated, driving the backend's `WasiXxxCtx` against the real service.

Do not add tests that compile, deserialize, or instantiate a WASM guest.

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
        let answer = Answer { value: self.0.clone(), usage: None, transcript: None };
        async move { Ok(answer) }.boxed()
    }
}
```

Drive that impl against `WasiModelCtx` in a unit test; do not bind it through a guest.

## Running

```bash
cargo make test                                   # `cargo nextest run --locked --all --all-features`
cargo test --doc --all-features --workspace       # doc tests
```

`cargo-nextest` must be installed with `--locked` (`cargo install --locked cargo-nextest`).

## Naming

A test name is the scenario (`set_then_get`), not a restated expectation (`set_then_get_round_trips`).
