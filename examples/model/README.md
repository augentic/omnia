# Model completion (replay)

Proves the replay seam of the `wasi-model` boundary: a guest calls `create` across the `omnia:model/completion` boundary and receives a **validated, deterministic** answer from the in-tree `ModelDefault` (replay) backend — no live model, no network.

## What it shows

- `guest` ([`guest.rs`](guest.rs)) **imports** `omnia:model/completion` and exposes an async `run`. It builds a `json-schema` prompt, assembling the `system` / `messages` channels with the guest-side `Sections` builder (role / task / context), sets `grants.references = "shelf"` as data, reads the preopen table via `wasi:filesystem/preopens` and lends the workspace named `.` through `grants.workspace`, then calls `create(request).await`.
- [`runtime.rs`](runtime.rs) binds the `WasiModel` host to `ModelDefault`, the replay backend that serves a recorded answer for an equivalent prompt.
- [`omnia.toml`](omnia.toml)'s `[[mount]]` preopens the repo root as a read-only workspace named `.`. The host resolves the lent descriptor back to that mount by directory identity; the replay backend ignores it (replay short-circuits tools).
- [`shelf.rs`](shelf.rs) is the `references` shelf: it exports `resolve` and is reached *only* via host-mediated dispatch when a backend follows `grants.references` (instance-per-call, no trigger). It is inert under the replay backend; the resolve path is proven by the integration test.
- [`omnia.toml`](omnia.toml) declares the `model` guest and the `shelf` guest.
- [`fixtures/`](fixtures) holds the checked-in replay fixture: the reduced, canonical prompt (the key) mapped to the validated answer.

```mermaid
flowchart LR
  guest["guest.run<br/>(imports completion)"] -->|"create(request)"| bind["create binding<br/>validation gate"]
  bind -->|"Request + ToolHost"| ctx["WasiModelCtx"]
  ctx --> replay["ModelDefault (replay)<br/>fixture lookup by canonical key"]
  replay -->|"validated answer"| guest
```

The runtime core stays generic (Law 2): no model id, provider, or schema dialect lives in Omnia. The boundary only ever hands the guest a **validated answer string**. The replay backend short-circuits tool calls, so this binary never emits a `resolve`; the host→guest `resolve` path (a fresh `shelf` instance per call) is exercised deterministically by the integration test, and live by the `omnia-genai` backend in the `backends` repo.

## Build the guests

A whole-workspace `wasm32-wasip2` build fails on the native-only host crates, so build the guest components explicitly:

```bash
cargo build -p examples --example model-wasm --target wasm32-wasip2
```

This emits `target/wasm32-wasip2/debug/examples/model_wasm.wasm` (the underscored name the manifest points at).

## Run

Point `MODEL_REPLAY_DIR` at the checked-in fixtures and start the host:

```bash
export RUST_LOG=info,opentelemetry_sdk=off
MODEL_REPLAY_DIR=examples/model/fixtures \
  cargo run --example model -- run --config examples/model/omnia.toml
```

Because the guest exports a plain async `run` (not an HTTP/messaging trigger), the end-to-end completion is exercised by the integration test rather than inbound traffic:

```bash
# after building the guest above (do NOT `cargo clean` in between):
cargo nextest run -p omnia-wasi-model --test replay
```

The test replays the guest through `ModelDefault` from the committed fixture — asserting the validated answer returns with no network. A second test (`resolve` path) drives a stub backend that calls `tool_host.resolve` for the `grants.references = "shelf"` prompt, proving the host→guest dispatch reaches a **fresh `shelf` instance per call** and the bytes round-trip — no network, fully in CI.

## Updating the fixture

If you change the guest's prompt, update the checked-in fixture under [`fixtures/`](fixtures) manually so its `key_request` matches the guest's reduced, canonical request (the assembled `system` / `messages` channels, not the pre-assembly template).
