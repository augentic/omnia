# Design: The `wasi-model` Host — Remaining Host-Side Work

> Status: Implementation plan — **remaining work only**. The `wasi-model` host core (the guest-visible `create` boundary, the `WasiModelCtx` backend trait, the `ToolHost` callbacks, structural answer validation, and composable record/replay) has landed in `crates/wasi-model`, together with the `omnia-genai` (frontier) and `omnia-cursor` (spawned-agent) backends and the in-tree `ModelDefault` (replay) backend in the `backends` repo. The `resolve` callback and its public host→guest dispatch entry point are wired. This document tracks only the host-side pieces that are **not yet built**.

The authoritative WIT now lives in `crates/wasi-model/wit/model.wit` at **`omnia:model@0.1.0`**. The 0.2.0 boundary defines:

- **`create` / `create-stream`** — single-shot and streaming completion entry points (guest calls `create`; the host binding delegates to the backend's `WasiModelCtx::complete`).
- **`format`** — a variant (`text`, `json`, `schema(schema)`) replacing the old `response-format` record + kind enum.
- **`grants`** — host capabilities lent per call (`references`, `workspace`, `verify`), replacing `tool-grants`.
- **`tool`** — a variant (`function(function)`, `mcp(mcp)`) for guest-declared functions and named MCP server grants (logical names resolved from deployment config; no URLs in the WIT).
- **`reply { answer, usage }`** — the validated answer plus optional token accounting returned from `create`.
- **`event`** — streaming deltas (`delta`, `done(reply)`, `failed(error)`), replacing `stream-event`.
- Typed **`role`**, **`generation`** (with `effort` / `seed` / `top-k`), and metadata as `list<tuple<string, string>>`.

Everything below is impl work behind that boundary.

## 1. `format::schema` validation gate

The `create` host binding validates every answer before the guest sees it, but only the **structural** gates are live: `format::json` (root is an object), `format::text` (root is a string), implemented with `serde_json` alone. The `format::schema(_)` arm is currently **parse-only** — it confirms the answer is valid JSON but does not enforce the embedded JSON Schema document.

Remaining: pick and pin a JSON-Schema validator crate, then turn on the schema gate in the `create` host binding (`crates/wasi-model/src/host/model_impl.rs`). This is the gate Specify's judgment operations require for typed decisions, so it is the highest-value remaining host-side item. Backends that self-check (genai's repair loop) already validate internally; this ensures the host validation gate enforces the schema too.

## 2. `create-stream` host binding

`create-stream` and the `event` variant are in the 0.2.0 WIT and the binding is generated, but the host impl returns `error::backend("streaming unsupported")`.

Remaining: wire the host-side production of a native `stream<event>` — the codebase's first native WIT `stream<>` (existing hosts model streams as resources). Streaming does not change validation: only the terminal `done(reply)` is schema-checked and recorded. This is a deliberate one-time exercise of host-side stream production, kept off the critical path until now.

## 3. Working-tree tools: `read` / `list` / `write`

The `ToolHost` methods `read` / `list` / `write` are **loud stubs**. They consume the wasi-filesystem working-tree host of [RFC-55](rfc-55-working-tree.md), which is **not yet built**, plus the `wasi:keyvalue` cross-turn session state that backs `write` → `read` visibility within one completion.

Remaining, once RFC-55 lands:

- Resolve the `prompt.grants.workspace` `borrow<descriptor>` against the resource table and back `read` / `list` with bounded reads through it (no OS path or descriptor reaches the model).
- Back `write` with host-held session state in `wasi:keyvalue`, keyed by the prompt hash, so an edit in one tool turn is visible to a `read` in the next. A leaked in-memory session is a regression.
- The lent `grants.workspace` descriptor already resolves to its `local-path` through the mount registry (`ToolHost::local_path`), which the cursor backend consumes today; RFC-55 adds only the bounded `read` / `list` / `write` faces above, not this path resolution.

The dispatch loop for these tools belongs to the genai backend; see [RFC-59](rfc-59-working-tree-tools.md). This section owns only the host-side wiring.

## 4. `verify` — routing only today

The runtime core routes a `verify(check)` against `prompt.grants.verify` and acknowledges it, but the profile definitions, sandboxing, and severity-tiered report mapping are owned by [RFC-60](rfc-60-verify-profiles.md) and are not implemented. No host-side work remains here beyond the routing already in place; the execution side lands in RFC-60.

## 5. Replay expansion and content-addressed keying

The minimal replay seam is live: `ModelDefault` serves a recorded answer for an equivalent prompt, keyed by canonical-JSON equality of the reduced prompt, from a directory of JSON fixtures.

Remaining (the [RFC-58](rfc-58-model-router-slm.md) replay *expansion*, tracked there):

- Content-addressed `sha256(canonical_json(key_prompt))` keying, replacing canonical-string equality.
- Fixture management, matching policy, and cross-backend diagnostics.
- `--output-format stream-json` transcript capture for richer recordings of spawned-agent runs.

## 6. References

- [RFC-59](rfc-59-working-tree-tools.md) — the genai backend's remaining `read` / `list` / `write` tool loop and host-held session state.
- [RFC-58](rfc-58-model-router-slm.md) — backend catalogue remainder (router, SLM) and the replay expansion.
- [RFC-55](rfc-55-working-tree.md) — the working tree's `descriptor` / `local-path` faces the working-tree tools depend on.
- [RFC-60](rfc-60-verify-profiles.md) — the verify profile definitions `verify` routes to.
- [guest-registry-transports.md](guest-registry-transports.md) — host-mediated dynamic linking; `resolve` rides it host-side (landed).
