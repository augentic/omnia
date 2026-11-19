# Model Interface Reference

Reference for the `omnia:model` package (version `0.2.0`): its interfaces, the session and its calls, the request, the checks the host enforces, the reply and `exhausted`, the errors, and the backend matrix. The walk-through is in [Model Completions and MCP](../guides/model-completions.md), and the authoritative WIT is [`crates/wasi-model/wit/model.wit`](../../crates/wasi-model/wit/model.wit).

- [Interfaces](#interfaces)
- [The session](#the-session)
- [Request](#request)
- [Host checks](#host-checks)
- [Reply and `exhausted`](#reply-and-exhausted)
- [Errors](#errors)
- [Backends implementing this interface](#backends-implementing-this-interface)

## Interfaces

```wit
// completion
open: async func(name: string) -> result<model, error>;

resource model {
  complete: async func(request: request, results: stream<call-result>)
    -> result<session, error>;
}
```

| Interface | Role |
| --------- | ---- |
| `types` | The types the others share: the `error`, the parts of a request, the calls and their results, the `reply`, and `exhausted`. |
| `completion` | `open` returns a handle on a model the deployment declares. An undeclared name fails with `not-found`, here or on the first completion. `complete` begins a completion. |

The `core` world holds `types` and `completion`, which every backend serves. The `imports` world includes `core`, and an optional interface joins it there.

Guests normally reach these through the `omnia_sdk::Model` trait, which `omnia_sdk::WasiProvider` implements on `wasm32` over the raw bindings at `omnia_wasi_model::omnia::{completion, types}`. `Model::complete` and `Model::complete_with` run the session and hand each tool call to a closure (see the [guide](../guides/model-completions.md)). Each maps to the WIT as follows:

- It opens `Request::model`, or `default` when that is `None`.
- A `Tool::Function` is a `tool`, and an empty description is `none`.
- An `Effort` is its level's name: `low`, `medium`, `high`, or an `Effort::Other` level as written.
- A check reaches the handler as a `ToolCall` named `check` (`model::CHECK_TOOL`) whose `arguments` are the candidate, and its answer goes back as a `verdict`. So a declared tool named `check` fails with `InvalidRequest`.

## The session

```wit
record session {
  calls: stream<call>,
  reply: future<result<result<reply, exhausted>, error>>,
}
```

The guest makes the `results` stream, keeps its writing end, and passes the reading end to `complete`. A request the host rejects fails `complete` itself, and any later failure arrives on `reply`.

- On `calls` the host asks the guest to run a tool, as a `tool-call` of `id`, `name`, and JSON `arguments`. When the request sets `check`, it also asks the guest to judge each candidate answer, as a `candidate` of `id` and `answer`.
- The guest answers each call by its `id` on `results`: a `tool-result` for a tool call, and a `verdict` for a check. Calls may be outstanding together, and the guest answers them in any order.
- A result that answers no outstanding call is dropped.
- A result of the wrong kind for its call fails the completion with `invalid`. So does closing `results` or dropping `calls` while a call is outstanding.
- `calls` ends when the completion does. The host always resolves `reply`, so a spent budget arrives as a value, never as a dropped writer.
- Dropping `reply` abandons the completion.

A `tool-result`'s `output` is the tool's output, or why it failed. The model reads either, and a failure leaves it free to call again. The host does not hold a call's `arguments` to the tool's `input-schema`.

### Session limits

The host holds every session to the backend's `Limits`, which `ModelClient::limits` returns:

| Limit | Default | Past it |
| ----- | ------- | ------- |
| `max_tool_calls` | 32 tool calls | `exhausted` |
| `tool_timeout` | 60 s for the guest to answer one call | `exhausted` |
| `max_result_bytes` | 1 MiB for a tool's output or failure, and for a correction | `invalid` |

A check is outside the tool-call budget, and needs no declaration in `tools`. The timeout applies to it, and the byte cap bounds its correction. There is no streaming variant of `complete` yet.

## Request

The guest names the model when it opens it, so a request carries none.

| Field | Type | Meaning |
| ----- | ---- | ------- |
| `system` | `option<string>` | Standing instructions, read before `messages`. |
| `messages` | `list<message>` | The conversation so far, oldest first. |
| `generation` | `option<generation>` | The controls on how the model generates. `none` takes the backend's defaults. |
| `format` | `format` | The shape the answer should take (see below). |
| `tools` | `list<tool>` | The tools the guest runs for the model. |
| `grants` | `grants` | What the guest lends the completion (see below). |
| `check` | `bool` | Whether the guest judges each candidate answer (see below). |

### `message`

`role` (`system` \| `user` \| `assistant`) plus `content`, the turn's text. The guest-side `Sections` builder assembles `system` and `messages` from structured fields (role, task, context, ...) so prompts stay consistent. See the [guide](../guides/model-completions.md#requesting-a-completion-from-a-guest).

### `generation`

`temperature`, `top-p`, `max-tokens`, `stop-sequences`, `seed`, and `effort`. `max-tokens` bounds one response, and a completion that calls tools takes several. An absent field takes the backend's default, and a backend carries every field given or fails with `unsupported`.

`effort` is how much the model reasons before it answers, by the level's name. `low`, `medium`, and `high` are portable. The host passes a level through as written, and a backend fails one it does not know with `invalid`.

### `format`

| Variant | Steering |
| ------- | -------- |
| `text` | Plain text. |
| `json` | Toward a JSON object. |
| `schema(schema)` | Toward the given JSON Schema. `schema` carries a `name` the provider knows it by (e.g. `verdict`), and the schema document as a JSON string. |

`format` steers only. Backends pass it to the provider as `response_format` where the provider constrains decoding, or as instruction prose where it does not. They read the candidate from the model's final text: the whole text when it parses as JSON, else the largest fenced or bracketed JSON value that parses whole, else the text as written. Nothing validates the answer against it. Acceptance is the guest's, through `check`.

### `check`

When `check` is set, the backend does not finish on the model's final text. It offers each candidate to the guest as a `check` call, and the guest answers with a `verdict`:

| `outcome` | Effect |
| --------- | ------ |
| `ok` | The candidate is the reply, and the completion ends. |
| `err(text)` | The model reads `text` as a correction, and proposes another candidate. |

A backend that runs out of candidates on a rejection ends the completion `exhausted`. Guests using `omnia-sdk` reach this through `Request::check` and a `complete_with` handler matching `call.name == model::CHECK_TOOL`. Or they use `model::Question<T>`, which derives the steering schema from `T`, deserializes each candidate, runs the guest's closure, and returns the accepted `T`.

### `tool`

| Field | Meaning |
| ----- | ------- |
| `name` | The name the model calls the tool by, given once in a request. A portable name is 1 to 64 characters long and holds only ASCII letters, digits, `_`, and `-`. |
| `description` | What the tool does, for the model to read. |
| `input-schema` | The JSON Schema of the tool's arguments. A portable one has a root type of `object`. |

The model calls a tool through the session's `calls`, and the guest runs it.

### `grants`

`workspace` is the directory the completion works in, an `option<location>`. A `location` is a `root`, a borrow of a `wasi:filesystem` directory descriptor the guest holds, plus a `subpath` beneath it. An empty subpath names the root itself. Being a typed resource borrow, a location cannot be forged. The host resolves it to an authorised mount by directory identity, so nothing outside a mount is reached. Backends then reach it as bounded `read`, `list`, and `write` through `ToolHost` (genai), or as the absolute local path (the cursor agent's working directory).

`worker` is what a backend's worker may reach beyond the workspace, an `option<worker-grant>`. A worker is a process the backend runs for the completion, such as a command the model asks for. The guest states the grant, because it knows the toolchain the completion runs:

- `egress` lists the host names the worker may connect to through the backend's proxy. Each is a whole name, or `*.suffix` for a domain and every host beneath it. None is an IP literal, and none carries a port, a scheme, or a path.
- `read` lists the directories the worker may read beside the workspace. Each is `~/<path>` beneath the operator's home, where `<path>` follows the rule of `location.subpath` and is not empty. A directory the machine lacks grants nothing.
- `env` lists the environment variables set for the worker. A name matches `[A-Za-z_][A-Za-z0-9_]*`, and a leading `~/` in a value stands for the operator's home.

A grant is a ceiling, so a backend that runs no worker honours it. What it opens on the machine is the backend's. The cursor and genai backends lay it on the kernel boundary around the worker each runs.

### Host-injected tools

The names `read`, `list`, and `write` are reserved for the host's workspace tools, and a backend may reserve others, as genai reserves `shell`. When the guest lends `grants.workspace`, the genai backend advertises `read` and `list` to the model, and executes them host-side through the `ToolHost` workspace capability. These are bounded reads and listings that never traverse the session. A read result must be UTF-8 text and, like a session tool result, fit the per-result byte cap. `write` stays reserved and served by `ToolHost`, but is not yet advertised to the model. The cursor backend advertises none of these: its agent inspects the workspace natively through the local path.

Declared function tools travel a different road. The backend forwards the model's call through `ToolHost::call_tool`, the host checks the name against the request's declared tools and enforces the session limits, and the guest's handler answers over the session streams.

## Host checks

The host makes these checks whatever the backend. Each fails the call it applies to, before the backend sees the request:

- `open` with an empty name is `invalid`.
- A request whose messages hold no text is `invalid`.
- A tool named `read`, `list`, or `write` is `invalid`, and so is a name given twice or an `input-schema` that is not JSON.
- A `format::schema` whose schema is not JSON is `invalid`.
- A `worker` grant without a `workspace` is `invalid`, and so is an entry that breaks its shape.
- A `workspace` whose root is not one of the guest's mounts, or is not a directory, is `denied`.
- A `workspace` subpath with an empty, `.`, or `..` segment, or a `\`, is `invalid`, and one that does not exist is `not-found`.

While a completion runs, the host fails it on `reply`:

- with `other` when the model calls a tool the request does not declare;
- with `exhausted` when the tool-call budget runs out, or the guest takes longer than the timeout to answer a call;
- with `invalid` for a tool's output or failure, or a correction, over the byte cap;
- with `invalid` for a result of the wrong kind, or `results` closed or `calls` dropped while a call is outstanding.

The first of these to happen is the completion's outcome, whatever the backend answers after it.

## Reply and `exhausted`

A completion that has an answer resolves `reply` to `ok(ok(reply))`:

| Field | Type | Meaning |
| ----- | ---- | ------- |
| `answer` | `string` | The text the guest's check accepted, or the model's last text where the request set no check. |
| `usage` | `option<usage>` | The tokens the completion spent, `none` where the backend reports no usage. |

A `usage` counts over every response the model wrote:

- `input-tokens` are the tokens the model read, cached ones included;
- `output-tokens` are the tokens it wrote, reasoning included;
- `reasoning-tokens` is the part of `output-tokens` spent on reasoning;
- `cache-read-tokens` and `cache-write-tokens` are the parts of `input-tokens` read from and written to a cache.

The last three are `none` where the backend keeps no count.

A completion that spent a budget before it had an answer resolves `reply` to `ok(err(exhausted))`, an outcome rather than an error. Its `detail` says which budget ran out, for a person to read, and its `usage` is what the completion spent. The deployment sets the budgets. They bound:

- the responses the model writes, and the tokens in them;
- the tool calls a completion makes;
- the time the guest takes to answer a call;
- the time the completion takes.

## Errors

| Case | Meaning |
| ---- | ------- |
| `not-found` | The deployment does not declare the model, or the place lent does not exist. |
| `denied` | The provider refused the caller, or the guest lends a place it does not hold. |
| `invalid` | The request was rejected, such as a reserved tool name or a schema that is not JSON. The guest's side of the session fails this way too (see [Host checks](#host-checks)). |
| `unsupported` | A valid request this backend cannot serve, such as a `generation` field or a `format` it cannot carry. |
| `unavailable` | The provider did not answer, or turned the call away for now as a rate limit does. |
| `other` | Any other failure, such as a call to a tool the request does not declare. |

A backend picks a failure's case by returning an `omnia_wasi_model::Error`, and a spent budget by returning an `omnia_wasi_model::Exhausted`. Any other failure is `other`.

The SDK reports these as an `omnia_sdk::model::Error`:

- `invalid` is `InvalidRequest`.
- `exhausted` is `BudgetExhausted`. Its detail is the last correction when the last candidate was rejected, and the `exhausted` detail otherwise.
- `other` is `Backend`.
- Every other case is the variant of its name: `NotFound`, `Denied`, `Unsupported`, or `Unavailable`.

Each converts to an `omnia_sdk::Error` coded as `model::Error::code` names it: a `BadRequest` for `InvalidRequest` and `BudgetExhausted`, a `BadGateway` for `Unavailable` and `Backend`, and a `ServerError` for the rest.

## Backends implementing this interface

| Backend | Location | Notes |
| ------- | -------- | ----- |
| `ModelDefault` | in-tree (`wasi-model`) | Deterministic echo over the models `MODEL_NAMES` declares (comma-separated, default `default`). A `format::text` answer is the last message, and a `format::json` one wraps it as `{"echo": ...}`. A `format::schema` and a set `generation` field are `unsupported`. It honours `check` once, so a rejection is `exhausted`. |
| `omnia-genai` | omnia-backends repo | Provider APIs in-process. Drives the function-tool session loop and the `check` loop. Advertises `shell` over a lend beneath a configured shell root, bounded by `grants.worker`. |
| `omnia-cursor` | omnia-backends repo | Bridge-managed Cursor agent (`cursor-sdk-bridge`). Function tools bridge into the session as SDK custom tools. Workspace optional (tools-only in a private directory when unlent). 120s default inactivity timeout. |
