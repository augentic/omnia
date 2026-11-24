# Omnia WASI Model

This crate provides the Model interface for the Omnia runtime. The guest-side `Sections` prompt builder is `omnia_sdk::model::prompt`.

## Interface

Implements `omnia:model`, the interface the SDK's `model` module binds, over one
`ModelClient` per backend.

- A guest opens a model by name and begins completions through the handle
  `open` returns. A name the deployment does not declare fails with
  `not-found`. The SDK opens `default` for a request that names no model,
  and a backend receives the name as `Request::model`.
- A completion is a session. On its `calls` the host asks the guest to run
  a tool, or to judge a candidate answer when the request sets `check`, as
  the SDK's `Request::complete_checked` does. The guest answers each call
  by its `id` on `results`, in any order.
- The session's `reply` is the answer, `exhausted` when a budget ran out
  first, or an error.
- The host holds every session to the backend's `Limits`, by default:
  - 32 tool calls, after which the completion is `exhausted`;
  - 60 seconds for the guest to answer a call, after which it is
    `exhausted`;
  - 1 mebibyte for a tool's output or failure and for a correction, beyond
    which it is `invalid`.
- The host validates nothing about the answer. `format` steers the
  provider, and acceptance belongs to the guest's `check`.
- A backend picks a failure's case by returning an
  `omnia_wasi_model::Error`, and a spent budget by returning an
  `omnia_wasi_model::Exhausted` naming the `Budget` that ran out. Any
  other failure is `other`.
- The host passes `effort` through as the guest gave it, so a backend
  fails an `other` level it does not know with `invalid`.
- A workspace lend carries the `worker` grant, so a worker grant cannot
  be given without a lend.
- A guest whose deployment grants list `hosts` may state only the worker
  `egress` those cover, and a request reaching past them is `denied`.

## Backend

- **Default**: `ModelDefault` (echo). It answers every completion with its own prompt, so guest wiring runs deterministically with no live model. The last message comes back as a string for `format::text`, and wrapped as `{"echo": ...}` for `format::json`. It serves the models `MODEL_NAMES` lists, comma-separated, and `default` alone when it is unset. Every other name is refused with `not-found`. What an echo cannot honour is `unsupported`: a `format::schema` and a set `generation` field. A `check` request gets exactly one candidate, so a rejection is `exhausted`. Tests that need a schema answer or a tool turn script `omnia_test::host::ScriptedModel` instead.

- **Production**: [`omnia-genai`](https://github.com/augentic/omnia-backends/tree/main/crates/genai) (provider APIs via the genai SDK), [`omnia-cursor`](https://github.com/augentic/omnia-backends/tree/main/crates/cursor) (bridge-managed Cursor agent) — a one-line swap in the host, guests untouched (see the [Production Backends guide](https://github.com/augentic/omnia/blob/main/docs/guides/production-backends.md)).

## Usage

Add this crate to your `Cargo.toml` and use it in your runtime configuration:

```rust,ignore
use omnia_wasi_model::{ModelDefault, WasiModel};

omnia::runtime!({
    hosts: {
        WasiModel: ModelDefault,
    }
});
```

## License

MIT OR Apache-2.0
