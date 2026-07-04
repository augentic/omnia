# Design: The `wasi-model` Host — Remaining Host-Side Work

> Status: Implementation plan — **remaining work only**. The `wasi-model` host core (the guest-visible `create` boundary, the `WasiModelCtx` backend trait, the `ToolHost` callbacks, format validation including the JSON-Schema gate, and composable record/replay) has landed in `crates/wasi-model`, together with the `omnia-genai` (frontier) and `omnia-cursor` (spawned-agent) backends and the in-tree `ModelDefault` (replay) backend in the `backends` repo. The `resolve` callback and its public host→guest dispatch entry point are wired. This document tracks only the host-side pieces that are **not yet built**.

The authoritative WIT now lives in `crates/wasi-model/wit/model.wit` at **`omnia:model@0.1.0`**. The boundary defines:

- **`create`** — the single-shot completion entry point (guest calls `create`; the host binding delegates to the backend's `WasiModelCtx::complete`). Streaming (`create-stream` and its `event` variant) is YAGNI-commented in the WIT until a backend actually streams.
- **`request { model, system, messages, generation, format, tools, grants }`** — a plain provider-shaped request. Structured prompt templates are assembled guest-side into `system` / `messages` by the `Sections` builder in `omnia_wasi_model::prompt`; no template record crosses the boundary.
- **`format`** — a variant (`text`, `json`, `schema(schema)`) replacing the old `response-format` record + kind enum. All three arms are enforced at the host validation gate, including full JSON-Schema validation for `schema` (via the pinned `jsonschema` crate); backends share the same check through `check_answer` to drive their repair loops.
- **`grants`** — host capabilities lent per call (`references`, `workspace`, `verify`), replacing `tool-grants`.
- **`tool`** — a variant (`function(function)`, `mcp(mcp)`) for guest-declared functions and MCP server grants, each grant carrying its own endpoint `url`.
- **`reply { answer, usage }`** — the validated answer plus optional token accounting returned from `create`; `answer` is plain text for `format::text` and a JSON document otherwise.
- **`error`** — typed failures led by `invalid-request` (caller errors: empty `messages`, reserved tool names, uncompilable schema documents), distinct from retryable `backend` failures.
- Typed **`role`** and **`generation`** (with `effort` / `seed`).

Everything below is impl work behind that boundary.

## 1. Working-tree tools: `read` / `list` / `write`

The `ToolHost` methods `read` / `list` / `write` are **loud stubs**. They consume the wasi-filesystem working-tree host of [RFC-55](rfc-55-working-tree.md), which is **not yet built**, plus the `wasi:keyvalue` cross-turn session state that backs `write` → `read` visibility within one completion.

Remaining, once RFC-55 lands:

- Resolve the `request.grants.workspace` `borrow<descriptor>` against the resource table and back `read` / `list` with bounded reads through it (no OS path or descriptor reaches the model).
- Back `write` with host-held session state in `wasi:keyvalue`, keyed by the request hash, so an edit in one tool turn is visible to a `read` in the next. A leaked in-memory session is a regression.
- The lent `grants.workspace` descriptor already resolves to its `local-path` through the mount registry (`ToolHost::local_path`), which the cursor backend consumes today; RFC-55 adds only the bounded `read` / `list` / `write` faces above, not this path resolution.

The dispatch loop for these tools belongs to the genai backend; see [RFC-59](rfc-59-working-tree-tools.md). This section owns only the host-side wiring.

## 2. `verify` — routing only today

The runtime core routes a `verify(check)` against `request.grants.verify` and acknowledges it, but the profile definitions, sandboxing, and severity-tiered report mapping are owned by [RFC-60](rfc-60-verify-profiles.md) and are not implemented. No host-side work remains here beyond the routing already in place; the execution side lands in RFC-60.

## 3. Replay expansion and content-addressed keying

The minimal replay seam is live: `ModelDefault` serves a recorded answer for an equivalent request, keyed by canonical-JSON equality of the reduced request (the post-assembly `system` / `messages` channels — exactly what the provider sees), from a directory of JSON fixtures.

Remaining (the [RFC-58](rfc-58-model-router-slm.md) replay *expansion*, tracked there):

- Content-addressed `sha256(canonical_json(key_request))` keying, replacing canonical-string equality.
- Fixture management, matching policy, and cross-backend diagnostics.
- `--output-format stream-json` transcript capture for richer recordings of spawned-agent runs.

## 4. References

- [RFC-59](rfc-59-working-tree-tools.md) — the genai backend's remaining `read` / `list` / `write` tool loop and host-held session state.
- [RFC-58](rfc-58-model-router-slm.md) — backend catalogue remainder (router, SLM) and the replay expansion.
- [RFC-55](rfc-55-working-tree.md) — the working tree's `descriptor` / `local-path` faces the working-tree tools depend on.
- [RFC-60](rfc-60-verify-profiles.md) — the verify profile definitions `verify` routes to.
- [guest-registry-transports.md](guest-registry-transports.md) — host-mediated dynamic linking; `resolve` rides it host-side (landed).
