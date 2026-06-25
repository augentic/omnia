# Design: The `wasi-model` Host & Its Backends (cursor-agent + genai)

> Status: Implementation plan. The Omnia-side design for the "Judgment: the `wasi-model` host" section of
> [architecture.md](architecture.md), realising [RFC-53](rfc-53-wasi-model.md) (the boundary),
> [RFC-59](rfc-59-model-tool-loop.md) (the tool loop), and the first two backends of
> [RFC-58](rfc-58-model-backends.md). The `resolve` callback rides the host-mediated dynamic linking
> mechanism designed in [guest-registry.md](guest-registry.md).

## 0. What we are building (and why it is three things)

The architecture sketch bundles judgment-as-an-effect into one `eval` call, but the implementation has three
separable layers. We treat them as layers because each is independently valuable and the next is built on the
one below.

1. **The `wasi-model` host core — the boundary.** Omnia exposes a `wasi-model` host whose `eval` export a
   guest calls to have a prompt evaluated (`eval: func(prompt) -> result<answer, error>`). This layer owns
   *only* the seam: the prompt / answer / error records, the `ModelBackend` trait behind `eval`, answer
   validation against the operation's expected schema, and the minimal record/replay seam. No model id, no
   vendor SDK, no tool loop. This is [RFC-53](rfc-53-wasi-model.md), and it is independently valuable: a guest
   can call `eval` and get a validated typed answer (or a deterministic replayed one) before any real model or
   tool exists.

2. **The model tool loop.** Inside one `eval`, a backend may drive a model through a typed tool surface —
   `resolve` (follow a brief's references), `read` / `list` / `write` (the working tree), `verify` (a closed
   profile) — accumulating edits in host-held state and repairing until the answer validates. This is
   [RFC-59](rfc-59-model-tool-loop.md). It is built on Layer 1's boundary and on the guest registry: `resolve`
   *is* host-mediated dynamic linking ([guest-registry.md](guest-registry.md) §4) invoked from the host side.

3. **The backends.** Behind the trait sit the swappable model backends. We build two first —
   **genai** (frontier / hosted, an in-process tool loop) and **cursor-agent** (a spawned, filesystem-capable
   agent that owns its own loop) — plus the **replay** backend that Layer 1 already seeds, expanded into a
   production fixture store. This is the first slice of [RFC-58](rfc-58-model-backends.md).

Keeping these layered matters for sequencing: Layer 1 is a self-contained host crate with no model dependency
(its default backend is replay); Layer 2 adds the tool loop and takes a dependency on the guest registry for
`resolve`; Layer 3 adds the two real model dependencies (`genai`, the `cursor-agent` CLI) strictly behind the
trait, so the floor never learns a model id (Law 2).

## 1. Goals and non-goals

### Goals

- A **domain-agnostic** `wasi-model` host in the Omnia floor: it knows the *shape* of judgment (a typed
  prompt in, a validated typed answer out) and the *mechanism* (the backend trait, the tool loop, record /
  replay) — never which model, which provider, or any Specify concept (Law 2 in
  [architecture.md](architecture.md#the-four-laws)).
- **`eval` is a typed effect like any other host call** — a guest treats it exactly like
  `wasi:keyvalue.get`: a typed call whose backend it never sees ([architecture.md](architecture.md#judgment-the-wasi-model-host)).
- **The model id and vendor SDK live only in the backend.** `genai`'s `Client`, the `cursor-agent` process,
  and every API key sit below the `ModelBackend` boundary; nothing vendor-specific rises above it
  ([RFC-53](rfc-53-wasi-model.md) risks).
- **Instance-per-call preserved through the callback.** A `resolve` lands in a *fresh* adapter instance, so
  the model's reference resolution can never recursively re-enter the guest that called `eval`
  ([architecture.md](architecture.md#resolving-references--the-host-calls-back-into-a-guest)).
- **Validated answers only.** A model response that does not validate against the operation's expected schema
  is a repair-loop input or a backend failure — never a guest-visible answer ([RFC-53](rfc-53-wasi-model.md)).
- **Boundary-level record / replay.** Recording and replay wrap the typed prompt / answer boundary, so CI is a
  backend swap (replay) and never depends on a live model — including for the spawned-agent backend that owns
  its own loop ([RFC-53](rfc-53-wasi-model.md) / [RFC-58](rfc-58-model-backends.md)).
- **Two real backends behind one seam**, selected by deployment config: `genai` (frontier) and `cursor-agent`
  (spawned agent), both recordable through the same boundary.
- Follows the existing host-crate shape exactly (`crates/wasi-keyvalue` is the template), so `wasi-model`
  drops into the `runtime!` macro as `WasiModel: <Backend>` with no new runtime machinery.

### Non-goals (for this work)

- The `augentic:specify` WIT package and the concrete brief / answer *types*. The floor defines the *generic*
  prompt / answer envelope; Specify projects its operation-specific schemas onto it (§3.2). Those schemas live
  in the Specify consumer.
- The **router** backend (select a backend per call by difficulty / mode), the **local SLM** backend, and the
  full replay-fixture management beyond the minimal seam — the rest of [RFC-58](rfc-58-model-backends.md).
- Closed **verify profile** definitions, sandboxing, and severity mapping — [RFC-60](rfc-60-verify-profiles.md).
  We design the `verify` tool seam; the profiles are landed there.
- The guest registry and host-mediated dynamic linking themselves — designed in
  [guest-registry.md](guest-registry.md). This RFC *consumes* that mechanism for `resolve`; it does not
  rebuild it.

## 2. Where this lands in the current code

`wasi-model` is a new host crate that follows the established `omnia-wasi-*` shape exactly. The template is
`crates/wasi-keyvalue`: a `WasiKeyValue` host struct that implements `Host<T>` (`add_to_linker`) and
`Server<S>` (a no-op `run`), a `WasiKeyValueView` trait the `Linker<T>` type implements, a `WasiKeyValueCtx`
trait the *backend* implements, a `KeyValueDefault` backend implementing `Backend` (env-driven `connect`), and
an `omnia_wasi_view!` macro the `runtime!` expansion calls.

```46:99:crates/wasi-keyvalue/src/host.rs
/// Host-side service for `wasi:keyvalue`.
#[derive(Debug)]
pub struct WasiKeyValue;

impl<T> Host<T> for WasiKeyValue
where
    T: WasiKeyValueView + 'static,
{
    fn add_to_linker(linker: &mut Linker<T>) -> anyhow::Result<()> {
        store::add_to_linker::<_, Self>(linker, T::keyvalue)?;
        // ...
    }
}

/// A trait which provides internal WASI Key-Value context.
pub trait WasiKeyValueCtx: Debug + Send + Sync + 'static {
    fn open_bucket(&self, identifier: String) -> FutureResult<Arc<dyn Bucket>>;
}
```

So a guest declares `WasiModel: GenaiBackend` (or `CursorAgentBackend`, or `ReplayBackend`) in `runtime!`
alongside the host backends it already names, and the macro wires it in — no new runtime plumbing:

```rust
omnia::runtime!({
    main: true,
    hosts: {
        WasiFilesystem: GitWorkingTree,
        WasiKeyValue:   KeyValueDefault,
        WasiModel:      GenaiBackend,   // <- swap to CursorAgentBackend / ReplayBackend by config
    }
});
```

Two differences from a plain effect host shape the design:

- **`wasi-model` is not purely guest→host.** Its tool loop calls *back* into guests (`resolve`) and into other
  host services (the working tree via `wasi:filesystem`, session state via `wasi:keyvalue`). So a backend
  needs a host-provided handle to those capabilities. We give it one — a `ToolHost` (§4.2) — rather than
  letting backends reach the registry directly, keeping each backend a pure "drive a model, call typed tools"
  unit.
- **The `resolve` callback needs the guest registry.** Resolving a brief reference instantiates the adapter
  guest fresh and calls its `references` export. That is precisely host-mediated dynamic linking
  ([guest-registry.md](guest-registry.md) §4) — only invoked from the host side, not from a guest import. So
  Layer 2 depends on the registry landing first; Layer 1 does not.

## 3. Layer 1 — The `wasi-model` host core (the boundary)

### 3.1 The WIT

The floor owns a small, generic interface. The guest hands over a complete, self-contained prompt and gets
back a validated answer or a typed error — never a transcript, never a model id.

```wit
// wit/model.wit — the generic judgment effect (augentic:model, the floor's own package)
package augentic:model@0.1.0;

interface judgment {
    /// An opaque, content-addressable identity for the brief/operation being
    /// judged. The floor never parses it; a consumer (Specify) projects its
    /// operation scheme onto it. Mirrors GuestId in guest-registry.md §3.1.
    type brief-id = string;

    /// What the caller wants judged: the whole brief, the operation kind, the
    /// expected answer shape (a schema the host validates against), and the
    /// handles the backend is allowed to expose as tools (working-tree cap,
    /// reference shelf identity). Untyped corpora never cross — only handles.
    record prompt {
        brief: brief-id,
        operation: string,            // opaque to the floor (e.g. "build", "review")
        answer-schema: schema,        // JSON Schema the host validates the answer against
        tools: tool-grants,           // which tools the backend may use this eval
    }

    record tool-grants {
        references: option<brief-id>, // adapter whose `references` shelf `resolve` targets
        working-tree: option<u32>,    // resource handle id for read/list/write, if lent
        verify: list<string>,         // allowed verify profile names
    }

    type schema = string;             // JSON Schema document
    type answer = string;             // JSON instance, validated against `schema`

    variant error {
        invalid-answer(string),       // backend produced output that never validated
        budget-exhausted(string),     // iteration / token / time / verify budget hit
        tool-failed(string),          // a non-repairable tool error
        backend(string),              // transport / process / provider failure
    }

    eval: func(prompt: prompt) -> result<answer, error>;
}

world model {
    export judgment;
}
```

The envelope is deliberately generic: `operation` is an opaque string, `answer-schema` is a JSON Schema, and
`answer` is a JSON instance. **The floor validates structure, not meaning** — it checks the answer parses and
conforms to the schema the prompt carried, and knows nothing of `build` vs `review`. Specify ships the
concrete schemas; the floor enforces them. (Whether the typed contract is JSON-Schema-over-strings or
generated WIT records is a consumer choice tracked in §7.3 — the floor only needs "a schema it can validate an
answer against".)

### 3.2 Prompt / answer / error records (host side)

Mirroring the generated-bindings pattern in `wasi-keyvalue/src/host.rs`, the host crate runs
`wasmtime::component::bindgen!` over `wit/` and re-exports the generated `prompt` / `answer` / `error` types.
The host-internal representation a backend sees is a thin owned mirror so backends never touch wasmtime types:

```rust
/// What a backend is asked to judge. Pure data — no wasmtime, no model id.
pub struct Prompt {
    pub brief: BriefId,
    pub operation: String,
    pub answer_schema: serde_json::Value,   // parsed JSON Schema
    pub grants: ToolGrants,
}

/// A backend's result before floor validation. The floor validates `answer`
/// against `prompt.answer_schema` and records `(prompt, transcript) -> answer`.
pub struct Candidate {
    pub answer: serde_json::Value,
    pub transcript: Transcript,             // tool-call log for record/replay (§3.4)
}
```

### 3.3 The `ModelBackend` trait

This is the `WasiModelCtx`-equivalent — the trait a backend implements, exactly where `WasiKeyValueCtx` sits in
the keyvalue crate. It carries no vendor type:

```rust
/// Implemented by every model backend (genai, cursor-agent, replay). The floor
/// hands it a prompt and a `ToolHost` (§4.2) and gets back a candidate answer
/// plus a recordable transcript. The floor — not the backend — validates the
/// answer and applies record/replay.
pub trait ModelBackend: Debug + Send + Sync + 'static {
    fn eval(
        &self,
        prompt: Prompt,
        tools: Arc<dyn ToolHost>,
    ) -> FutureResult<Candidate>;
}
```

Like `WasiKeyValueCtx`, a backend also implements `Backend` (the env-driven `connect` / `ConnectOptions` /
`FromEnv` pattern from `crates/omnia/src/traits.rs`) so `runtime!` can connect it concurrently at startup. The
`WasiModel` host struct implements `Host<T>` (binds `eval` onto the linker) and `Server<S>` (a no-op `run` —
`wasi-model` is purely linked, not a trigger server).

### 3.4 Answer validation and record/replay are floor decorators

Per [RFC-53](rfc-53-wasi-model.md), validation and replay are *host* concerns that wrap *any* backend, not
behaviour each backend re-implements. We model them as decorators over `ModelBackend`:

```text
guest --eval--> [Record] -> [Validate] -> <selected ModelBackend>            (live + recording)
guest --eval--> [Replay]                                                     (CI / deterministic)
```

- **`Validate`** parses the backend's `Candidate.answer` and checks it against `prompt.answer_schema`. A
  non-conforming candidate is *not* returned to the guest: it is fed back into the backend's repair loop
  (§4.3) until it validates or a budget is hit, at which point `eval` returns `error::invalid-answer` /
  `error::budget-exhausted`. The guest only ever sees a validated answer or a typed error.
- **`Record`** logs `(Prompt + Transcript) -> validated answer` to a fixture store keyed by a stable hash of
  the prompt.
- **`Replay`** *is* the default backend (§5.4) — it serves a recorded answer for an equivalent prompt, so CI
  runs a real `eval` path with no live model. This is why Layer 1 is independently shippable: `WasiModel:
  ReplayBackend` is a complete, testable host before genai or cursor-agent exist.

Because the decorators sit at the typed boundary, the spawned-agent backend (which owns its own loop and may
never call a single floor tool) is recorded and replayed identically to the in-process genai backend — the
recording captures what crossed `eval`, not how the backend got there.

## 4. Layer 2 — The model tool loop

### 4.1 The tool surface

Within one `eval`, a backend may expose these tools to the model ([RFC-59](rfc-59-model-tool-loop.md) §"The
tool surface"). The floor *implements* them; the backend *advertises and dispatches* them to the model:

- **`resolve(reference)`** — follow a brief's internal reference. The floor selects the adapter named by
  `prompt.grants.references`, instantiates it fresh, and calls its exported `references` shelf — host-mediated
  dynamic linking ([guest-registry.md](guest-registry.md) §4), instance-per-call.
- **`read(path)` / `list(path)`** — inspect the working tree through the capability lent in
  `prompt.grants.working-tree`. The model sees bounded results, never an OS path or a `descriptor`.
- **`write(path, bytes)`** — accumulate an edit against the session's base tree. Pending edits live in
  host-held state (`wasi:keyvalue`), not guest memory.
- **`verify(check)`** — request a closed verification profile from `prompt.grants.verify`. The profile
  definitions and sandboxing are owned by [RFC-60](rfc-60-verify-profiles.md); this design only routes the
  call.

### 4.2 `ToolHost` — the floor's tool implementations, handed to the backend

The backend never touches the registry, the working tree, or kv directly. The floor builds a per-`eval`
`ToolHost` and hands it to `ModelBackend::eval`. This is the one place the model tool loop binds to the rest of
the runtime:

```rust
/// The tool surface the floor implements for one `eval` and lends to the backend.
/// Each method is a typed callback; the backend turns model tool-calls into these.
pub trait ToolHost: Send + Sync {
    /// `resolve` — host-mediated dynamic linking into the adapter's `references`
    /// export (guest-registry.md §4). Always a fresh instance: a resolve cannot
    /// recursively re-enter the guest that called `eval`.
    fn resolve(&self, reference: Reference) -> FutureResult<Vec<u8>>;

    /// Bounded working-tree access via the lent wasi:filesystem capability.
    fn read(&self, path: String) -> FutureResult<Vec<u8>>;
    fn list(&self, path: String) -> FutureResult<Vec<DirEntry>>;
    fn write(&self, path: String, bytes: Vec<u8>) -> FutureResult<()>;

    /// Route a verify request to a closed profile (RFC-60).
    fn verify(&self, check: String) -> FutureResult<VerifyReport>;
}
```

The floor constructs the `ToolHost` from the session (§4.4): it captures the registry handle (for `resolve`),
the working-tree capability id from `prompt.grants.working-tree`, the kv handle for accumulating `write`s, and
the allowed verify profiles. A backend that ignores tools (cursor-agent, §5.3) simply never calls these.

### 4.3 The repair loop

The backend drives the model until one terminal state ([RFC-59](rfc-59-model-tool-loop.md) §"Repair loop"):

- the answer validates against `prompt.answer_schema` (the floor's `Validate` decorator, §3.4) and returns
  through `eval`;
- a tool call fails with a typed, non-repairable error → `error::tool-failed`;
- an iteration / token / time / verify budget is exhausted → `error::budget-exhausted`;
- the backend records a failure answer for replay diagnostics.

Invalid candidates are loop inputs, never guest-visible. The loop is budgeted so it fails clearly rather than
spinning. Budgets reuse the existing per-call controls (`crates/omnia/src/options.rs`): the enclosing `eval`
host call already runs inside the guest's `guest_timeout` and epoch yielding, and `resolve` dispatches inherit
the dispatch-depth bound from [guest-registry.md](guest-registry.md) §6.6 — an `eval`→`resolve`→(adapter that
calls `eval`) chain is depth-counted like any other host-mediated dispatch.

### 4.4 Session state lives in `wasi:keyvalue`

Because guests are instance-per-call, one model session's durable state — the prompt and expected type, the
adapter identity in scope, the base `revision`, the working-tree capability, accumulated edits, and verify
results feeding the repair loop — lives in a host service, not guest memory
([RFC-59](rfc-59-model-tool-loop.md) §"Session state"). The floor keys session state in `wasi:keyvalue` under
the prompt hash, so a `write` during one tool turn is visible to a `read` in the next, and a leaked in-memory
session is a regression. This is also what makes the spawned-agent backend's local-path edits and the
in-process backend's `write` tool converge on the same "accumulated change-set" extraction at the end of
`eval`.

## 5. Layer 3 — The backends

All three backends implement the *same* `ModelBackend` trait (§3.3) and `Backend` (env-driven connect), and
are selected purely by what the deployment names in `runtime!` (or, later, the router of
[RFC-58](rfc-58-model-backends.md)). The floor's `Validate` / `Record` / `Replay` decorators (§3.4) wrap all of
them uniformly, so each backend only has to "produce a candidate answer + transcript".

Two shapes of backend fall out of the architecture, and the two we build are one of each:

- **In-process tool loop** — the backend owns the model API and turns the model's tool calls into `ToolHost`
  callbacks. `genai` is this (§5.2). The working tree is reached through the *bounded* `read`/`list`/`write`
  tools (no OS path crosses to the model).
- **Spawned, filesystem-capable agent** — the backend spawns an external agent that owns its own loop and
  reads/writes the working tree directly through the node-local `local-path` it is lent
  ([architecture.md](architecture.md#the-working-tree), [RFC-55](rfc-55-working-tree.md)). `cursor-agent` is
  this (§5.3). It still returns a validated answer through the same boundary and stays recordable.

### 5.1 Backend trait, restated for the two

```rust
// crates/wasi-model/src/host/genai_impl.rs        -> GenaiBackend
// crates/wasi-model/src/host/cursor_agent_impl.rs -> CursorAgentBackend
// crates/wasi-model/src/host/replay_impl.rs        -> ReplayBackend (also the Default)
```

Each is a `#[derive(Clone)]` handle (cheap to clone into each per-call store, like `KeyValueDefault`), holds
its connection/config, and implements `ModelBackend::eval`.

### 5.2 `GenaiBackend` — frontier / hosted, in-process tool loop

[`genai`](https://github.com/jeremychone/rust-genai) (`genai = "0.6"`) is one ergonomic Rust API over 25+
providers (OpenAI, Anthropic, Gemini, Ollama, …), so "switch frontier / hosted / local providers" is backend
config, never a contract change ([RFC-58](rfc-58-model-backends.md)). The model id and provider live entirely
here.

**Connect.** `ConnectOptions` reads the model id and provider auth from env (`OMNI_MODEL`, plus the provider's
own `OPENAI_API_KEY` / `ANTHROPIC_API_KEY` / … that `genai` already honours). `connect` builds a `genai::Client`
once at startup:

```rust
pub struct GenaiBackend {
    client: genai::Client,   // holds provider auth; never crosses the boundary
    model: Arc<str>,         // e.g. "gpt-5.5", "claude-...", "gemini-..."
}

impl Backend for GenaiBackend {
    type ConnectOptions = GenaiOptions;   // model + provider auth, FromEnv
    async fn connect_with(opts: GenaiOptions) -> Result<Self> { /* build Client */ }
}
```

**`eval` — the in-process loop.** Map the typed surface onto `genai`'s chat API and run the tool-use loop:

1. Build a `ChatRequest` with a system prompt derived from `prompt.operation`, the brief as a user message,
   and `with_tools([...])` advertising `resolve` / `read` / `list` / `write` / `verify` as `genai::Tool`s whose
   schemas mirror the `ToolHost` methods. Set `ChatResponseFormat::JsonSpec(JsonSpec::new(prompt.answer_schema))`
   so the final answer is structured output validated against the prompt's schema.
2. `client.exec_chat(&self.model, req, None).await`. If `response.into_tool_calls()` is non-empty, dispatch
   each through the `ToolHost` callback, append the tool responses to the request (genai's
   `append_tool_use_from_stream_end` / tool-response messages), and loop.
3. When the model returns content instead of tool calls, that content is the candidate JSON answer. Return
   `Candidate { answer, transcript }` where `transcript` is the captured tool-call log (for `Record`).
4. The floor's `Validate` decorator (§3.4) checks the answer against the schema; a miss re-enters the loop with
   the validation error appended, up to the repair budget.

`resolve` is the interesting tool: dispatching it calls `ToolHost::resolve`, which the floor satisfies by
host-mediated dynamic linking into the in-scope adapter's `references` export — a *fresh* adapter instance,
isolated from the guest that called `eval`. The model never holds a descriptor or path; `read`/`list`/`write`
return bounded results from the lent working-tree capability.

### 5.3 `CursorAgentBackend` — spawned, filesystem-capable agent

This is the **spawned-agent** backend of [RFC-58](rfc-58-model-backends.md): the native layer spawns a fresh,
context-free `cursor-agent` session, hands it the brief, lets it own its own tool loop and read/write the
working tree directly through the `local-path` it is lent ([RFC-55](rfc-55-working-tree.md)), and parses a
validated answer back. It still returns through the [RFC-53](rfc-53-wasi-model.md) boundary and stays
recordable.

**Why a different shape.** A filesystem-capable agent cannot use the bounded `read`/`list`/`write` tools — it
needs real OS paths. That is exactly the `local-path` face of the working tree: an absent `local-path` is a
clean capability signal that an agent-driven build is unavailable on this node
([architecture.md](architecture.md#the-working-tree)). So `eval` on this backend checks
`prompt.grants.working-tree` resolves to a `local-path`; if not, it returns `error::backend("no local tree on
this node")`.

**Connect.** `ConnectOptions` reads `CURSOR_API_KEY` (or relies on a prior `cursor-agent login`) and an
optional `OMNI_MODEL`; `connect` just validates the `cursor-agent` binary is on `PATH`. No long-lived process —
each `eval` spawns a fresh, context-free session (no leaked transcript, per [RFC-58](rfc-58-model-backends.md)
risks).

**`eval` — spawn, run to completion, parse.** Using the documented headless surface
([Cursor CLI docs](https://cursor.com/docs/cli/headless)):

```bash
cursor-agent --print --force \
  --output-format json \
  --model "$OMNI_MODEL" \
  --workspace "$LOCAL_PATH" \
  "<brief + 'emit a final JSON answer conforming to this schema: …'>"
```

- `-p/--print` runs non-interactive to completion; `--force` (a.k.a. `--yolo`) grants write access so the
  agent can edit the working tree in place; `--workspace "$LOCAL_PATH"` scopes it to the lent tree;
  `--output-format json` emits a single JSON object (with the aggregated final `result`) on success and *no*
  well-formed object on failure — a clean success/failure signal.
- The backend parses `.result` as the candidate answer. (Operationally, `cursor-agent --print` is known to
  occasionally hang after completing, so the spawn is wrapped in a wall-clock timeout that maps to
  `error::budget-exhausted`; the existing per-call `guest_timeout` is the outer bound.)
- The agent's edits land directly in the `local-path` tree; the floor extracts the content-addressed
  change-set from that tree afterwards (the git-aware `wasi:filesystem` backend, [RFC-55](rfc-55-working-tree.md)),
  exactly as the `build` lifecycle expects ([architecture.md](architecture.md#lifecycle-of-an-operation)).
- `--output-format stream-json` (line-delimited events with stable session ids) is available if we later want
  per-tool-event progress in the recorded transcript; the `json` single-object form is enough to start.

The model id, the API key, and the entire agent protocol stay inside this backend; the calling guest sees the
same typed `answer` it would from genai or replay.

### 5.4 `ReplayBackend` — the default, deterministic backend

Replay belongs at the `wasi-model` boundary because it is the test substitute for judgment itself
([RFC-53](rfc-53-wasi-model.md)). `ReplayBackend` is the crate's **default** (the `KeyValueDefault` analogue):
with no API key and no spawned process, it serves the recorded answer for an equivalent prompt and lets one
vertical operation run deterministically in CI without a live model.

- **Fixtures** are the `(Prompt + Transcript) -> validated answer` rows that the `Record` decorator (§3.4)
  writes, keyed by a stable hash of the prompt. The minimal seam here is a directory of JSON fixtures
  (`OMNI_REPLAY_DIR`); the full fixture management, matching policy, and cross-backend diagnostics are the
  [RFC-58](rfc-58-model-backends.md) replay *expansion*, out of scope for this slice.
- **Determinism.** Because the record happens at the typed boundary, a fixture captured against `genai` or
  `cursor-agent` replays identically — CI never depends on which backend produced it.
- A prompt with no matching fixture returns `error::backend("no replay fixture")` (fail loud, never fall
  through to a live call).

## 6. Proof example (the acceptance vehicle)

A new `examples/model` with one tiny guest, proving the boundary and the loop with no Specify concepts:

- `judge` (guest) — imports `augentic:model/judgment`, builds a `prompt` whose `answer-schema` is a trivial
  shape (e.g. `{ "verdict": "pass" | "fail", "reason": string }`) plus `tools.references` naming a sibling
  `shelf` guest, and calls `eval(prompt)`; triggered via CLI.
- `shelf` (guest) — exports a `references` interface; `judge`'s prompt references one entry, so the model's
  `resolve` lands in a fresh `shelf` instance.

Three acceptance runs over the *same* example, swapping only the bound backend in `runtime!` / config:

1. **`WasiModel: ReplayBackend`** — with a checked-in fixture, `eval` returns the validated answer with no
   network. This is the CI gate (Layer 1).
2. **`WasiModel: GenaiBackend`** — against a real provider (gated on a key), the model emits a `resolve` tool
   call that dispatches into `shelf` (fresh instance, host-mediated linking), then returns a schema-valid
   answer; the `Record` decorator writes the fixture run 1 replays. This proves Layers 1+2+3a end-to-end.
3. **`WasiModel: CursorAgentBackend`** — given a `local-path` working tree, a spawned `cursor-agent` run
   returns a schema-valid `.result`; with no `local-path` it returns `error::backend`, proving the capability
   signal. This proves Layer 3b and the spawned-agent shape, with no guest or contract change between runs.
