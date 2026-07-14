# Model Interface Reference

Reference for the `omnia:model/completion` interface (version `0.1.0`): the request/reply types guests use and the validation the host enforces. The conceptual walk-through is in [Model Completions and MCP](../guides/model-completions.md); the authoritative WIT is [`crates/wasi-model/wit/model.wit`](../../crates/wasi-model/wit/model.wit).

## The function

```wit
create: async func(request: request) -> result<reply, error>;
```

One request, one validated reply. There is no streaming variant yet.

## Request

| Field | Type | Meaning |
| ----- | ---- | ------- |
| `model` | `option<string>` | Opaque model id hint, passed through unchanged; the backend may override. `None` defers entirely to the backend (genai defaults to its configured model). |
| `system` | `option<string>` | System/instructions channel. |
| `messages` | `list<message>` | Chat turns. **Must not be empty.** |
| `generation` | `option<generation>` | Sampling and length controls; omitted fields defer to backend defaults. |
| `format` | `format` | Required output shape (see below). |
| `tools` | `list<tool>` | Guest-declared functions and MCP grants. |
| `grants` | `grants` | Capabilities lent for this call (see below). |

### `message`

`role` (`system` \| `user` \| `assistant`) plus `content` (turn text). The guest-side `Sections` builder assembles `system`/`messages` from structured fields (role, task, context, ...) so prompts stay consistent — see the [guide](../guides/model-completions.md#requesting-a-completion-from-a-guest).

### `format`

| Variant | Answer contract |
| ------- | --------------- |
| `text` | Plain text. |
| `json` | Must parse as a JSON object. |
| `schema(schema)` | Must validate against the given JSON Schema. `schema` carries a `name` (passed to the provider, e.g. `verdict`) and the schema document as a JSON string. |

The **host** enforces the contract at the `create` gate: an answer that fails validation is never returned to the guest (backends may retry/repair internally; the host re-validates as the single authority).

### `generation`

`temperature`, `top-p`, `max-tokens`, `stop` (halt sequences), `seed`, and `effort` — a reasoning-effort hint (`minimal` \| `low` \| `medium` \| `high`) for thinking-capable models. All optional except `stop` (which may be empty).

### `tool`

| Variant | Fields | Support |
| ------- | ------ | ------- |
| `function` | `name`, `description`, `parameters` (JSON Schema for the arguments object) | Passed through to the provider (genai) |
| `mcp` | `name`, `tools` (allowlist; empty = all), `url` (server endpoint) | Cursor backend only; genai rejects MCP grants |

Function names must not collide with the reserved host-injected tool names below.

### `grants`

| Field | Type | Effect |
| ----- | ---- | ------ |
| `references` | `option<string>` | Guest id whose export the injected `resolve` tool dispatches to. |
| `workspace` | `option<borrow<descriptor>>` | A `wasi:filesystem` directory descriptor from the guest's own preopen table. Being a typed resource borrow, it cannot be forged — the host resolves it back to an authorized mount by directory identity, then exposes it to backends as bounded `read`/`list`/`write` (genai) or the absolute local path (cursor's `--workspace`). |
| `verify` | `list<string>` | Allowed verification profile names for the injected `verify` tool. |

### Host-injected tools

From the grants, the host — never the guest or backend — merges these tools into the completion: **`resolve`**, **`read`**, **`list`**, **`write`**, **`verify`**. Guests must not declare tools with these names (`invalid-request`). Backends execute them by calling back through the host's `ToolHost`, so every invocation passes host validation.

## Reply

| Field | Type | Meaning |
| ----- | ---- | ------- |
| `answer` | `string` | The validated answer, per `request.format`. |
| `usage` | `option<usage>` | Token accounting when the backend reports it: `input-tokens`, `output-tokens`, optional `reasoning-tokens`. |

## Errors

| Variant | Meaning | Retry? |
| ------- | ------- | ------ |
| `invalid-request(string)` | The request is malformed (empty `messages`, reserved tool name, invalid schema document). | Not without changing the request. |
| `invalid-answer(string)` | The backend never produced output that passed validation. | Possibly; the model may do better on retry. |
| `budget-exhausted(string)` | Iteration, token, time, or verify budget ran out. | With a larger budget. |
| `tool-failed(string)` | A tool call failed non-repairably. | Depends on the tool. |
| `backend(string)` | Transport, process, or provider failure. | Usually transient. |

## Backends implementing this interface

| Backend | Location | Notes |
| ------- | -------- | ----- |
| `ModelDefault` | in-tree (`wasi-model`) | Deterministic echo: text/json answer with the prompt; `format::schema` errors |
| `Scripted` | in-tree (`omnia-testkit`) | FIFO of scripted answers for tests and examples; never runs tools |
| `omnia-genai` | backends repo | Provider APIs in-process; function tools + injected `resolve`; no MCP |
| `omnia-cursor` | backends repo | Spawned `cursor-agent`; requires workspace grant; MCP via `.cursor/mcp.json`; 120s default timeout |
