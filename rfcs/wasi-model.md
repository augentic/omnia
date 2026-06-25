# Design: The `wasi-model` Host & Its Backends (cursor-agent + genai)

> Status: Implementation plan. The Omnia-side design for the "Judgment: the `wasi-model` host" section of [architecture.md](architecture.md), realising [RFC-53](rfc-53-wasi-model.md) (the boundary) and the first two backends of [RFC-58](rfc-58-model-backends.md). The genai backend's in-process tool loop is specified in [RFC-59](rfc-59-model-tool-loop.md). The `resolve` callback rides the host-mediated dynamic linking mechanism designed in [guest-registry.md](guest-registry.md).

## 0. What we are building (and why it is two things)

The architecture sketch bundles judgment-as-an-effect into one `eval` call, but the implementation has two separable layers. We treat them as layers because the boundary is independently valuable and backends are swappable behind it.

1. **The** `wasi-model` **host.** Omnia exposes a `wasi-model` host whose `eval` export a guest calls to have a prompt evaluated (`eval: func(prompt) -> result<answer, error>`). This layer owns *only* the seam: the prompt / answer / error records, the `WasiModelCtx` backend trait behind `eval`, answer validation against the operation's expected schema, and the minimal record/replay seam. No model id, no vendor SDK, no tool loop. This is [RFC-53](rfc-53-wasi-model.md), and it is independently valuable: a guest can call `eval` and get a validated typed answer (or a deterministic replayed one) before any real model exists.
2. **The backends.** Behind the trait sit the swappable model backends. We build two first — **genai** (frontier / hosted; drives an in-process tool loop via `[genai](https://github.com/jeremychone/rust-genai)`, specified in [RFC-59](rfc-59-model-tool-loop.md)) and **cursor-agent** (a spawned, filesystem-capable agent that owns its own loop) — plus the **replay** backend that Layer 1 already seeds, expanded into a production fixture store. This is the first slice of [RFC-58](rfc-58-model-backends.md).

Keeping these layered matters for sequencing: Layer 1 is a self-contained host crate with no model dependency (its default backend is replay).

Layer 2 adds the real backends strictly behind the trait, so the floor never learns a model id (Law 2).

## 1. Goals and non-goals



### Goals

- A **domain-agnostic** `wasi-model` host in the Omnia floor: it knows the *shape* of judgment (a typed prompt in, a validated typed answer out) and the *mechanism* (the backend trait, record / replay) — never which model, which provider, or any Specify concept (Law 2 in [architecture.md](architecture.md#the-four-laws)).
- `**eval` is a typed effect like any other host call** — a guest treats it exactly like `wasi:keyvalue.get`: a typed call whose backend it never sees ([architecture.md](architecture.md#judgment-the-wasi-model-host)).
- **The model id and vendor SDK live only in the backend.** `genai`'s `Client`, the `cursor-agent` process, and every API key sit below the `WasiModelCtx` backend boundary; nothing vendor-specific rises above it ([RFC-53](rfc-53-wasi-model.md) risks).
- **Instance-per-call preserved through the callback.** A `resolve` lands in a *fresh* adapter instance, so the model's reference resolution can never recursively re-enter the guest that called `eval` ([architecture.md](architecture.md#resolving-references--the-host-calls-back-into-a-guest)).
- **Validated answers only.** A model response that does not validate against the operation's expected schema is a backend failure — never a guest-visible answer ([RFC-53](rfc-53-wasi-model.md)). Backends that run a repair loop (genai; [RFC-59](rfc-59-model-tool-loop.md)) consume invalid candidates internally; the `eval` host binding is the floor's final validation gate at the boundary.
- **Boundary-level record / replay.** Recording and replay wrap the typed prompt / answer boundary, so CI is a backend swap (replay) and never depends on a live model — including for the spawned-agent backend that owns its own loop ([RFC-53](rfc-53-wasi-model.md) / [RFC-58](rfc-58-model-backends.md)).
- **Two real backends behind one seam**, selected by deployment config: `genai` (frontier) and `cursor-agent` (spawned agent), both recordable through the same boundary.
- Follows the existing host-crate shape exactly (`crates/wasi-keyvalue` is the template), so `wasi-model` drops into the `runtime!` macro as `WasiModel: <Backend>` with no new runtime machinery.



### Non-goals (for this work)

- The `augentic:specify` WIT package and the concrete brief / answer *types*. The floor defines the *generic* prompt / answer envelope; Specify projects its operation-specific schemas onto it (§3.2). Those schemas live in the Specify consumer.
- The **router** backend (select a backend per call by difficulty / mode), the **local SLM** backend, and the full replay-fixture management beyond the minimal seam — the rest of [RFC-58](rfc-58-model-backends.md).
- Closed **verify profile** definitions, sandboxing, and severity mapping — [RFC-60](rfc-60-verify-profiles.md). We design the `verify` tool seam; the profiles are landed there.
- The guest registry and host-mediated dynamic linking themselves — designed in [guest-registry.md](guest-registry.md). This RFC *consumes* that mechanism for `resolve`; it does not rebuild it.



## 2. Where this lands in the current code

`wasi-model` is a new host crate that follows the established `omnia-wasi-*` shape exactly. The closest templates are `crates/wasi-keyvalue` and `crates/wasi-blobstore` (the latter shows the current `Runtime` trait bound; `wasi-keyvalue` still uses the pre-rename `Runtime` and follows once that migration lands).

The shared shape is: a `WasiX` host struct implementing `HasData`, `Host<T>` (`add_to_linker`), and `Server<R>` (a no-op `run` for non-server hosts); a `WasiXView` trait the `Linker<T>` type implements; a `WasiXCtxView<'a>` carrying `ctx: &mut dyn WasiXCtx` + `table`.

A `WasiXCtx` trait the *backend* implements; an `XDefault` backend implementing `Backend` (env-driven `connect`) + `WasiXCtx`; and an `omnia_wasi_view!` macro the `runtime!` expansion calls.

```109:164:crates/wasi-blobstore/src/host.rs
/// Host-side service for `wasi:blobstore`.
#[derive(Debug)]
pub struct WasiBlobstore;

impl HasData for WasiBlobstore {
    type Data<'a> = WasiBlobstoreCtxView<'a>;
}

impl<T> Host<T> for WasiBlobstore
where
    T: WasiBlobstoreView + 'static,
{
    fn add_to_linker(linker: &mut Linker<T>) -> anyhow::Result<()> { /* generated add_to_linker */ }
}

impl<R> Server<R> for WasiBlobstore where R: Runtime {}   // non-server: default no-op `run`

/// Implemented by the backend (an in-memory store, a Redis store, …).
pub trait WasiBlobstoreCtx: Debug + Send + Sync + 'static {
    fn create_container(&self, name: String) -> FutureResult<Arc<dyn Container>>;
    // …
}
```

`wasi-model` is a **non-server** host — like `wasi-keyvalue` and `wasi-blobstore`, it is purely *linked* (its `Server::run` is the default no-op); only the trigger hosts (`wasi-http`, `wasi-messaging`, `wasi-websocket`) run servers. So its crate layout is near-identical to those two:

```text
crates/wasi-model/                      # the host crate, in the omnia repo
  wit/model.wit
  src/lib.rs                  # cfg split: guest (wasm32) vs host (native), as wasi-keyvalue
  src/host.rs                 # WasiModel, HasData, Host, Server, WasiModelView,
                              #   WasiModelCtxView, WasiModelCtx, omnia_wasi_view! macro,
                              #   the `generated` bindgen! module
  src/host/
    model_impl.rs             # impl the generated `eval` host binding -> calls ctx.eval(..)
    default_impl.rs           # ModelDefault (replay) — the KeyValueDefault analogue
```

As with `wasi-keyvalue` and its `KeyValueDefault`, the host crate ships the **default backend** (`ModelDefault`, replay) in-tree, while **real backends are separate** `omnia-<provider>` **crates in the** `backends` **repo** — exactly as `omnia-redis` / `omnia-nats` provide `WasiKeyValueCtx`. So the two model backends are `omnia-genai` and `omnia-cursor` (§5), each a host-only crate exposing `pub struct Client` that implements `Backend` + `WasiModelCtx`. A deployment binds whichever it wants in `runtime!`, exactly as `WasiKeyValue: KeyValueDefault`:

```rust
use omnia_genai::Client as Genai;       // model backend from the backends repo
// use omnia_cursor::Client as Cursor;  //   or the spawned-agent backend
// (omit the import to fall back to the in-tree ModelDefault replay backend)

omnia::runtime!({
    main: true,
    hosts: {
        WasiFilesystem: GitWorkingTree,
        WasiKeyValue:   KeyValueDefault,
        WasiModel:      Genai,          // <- swap to Cursor / ModelDefault by config
    }
});
```

The one structural difference from a plain effect host: `**wasi-model` is not purely guest→host.** During `eval`, a backend may call *back* into guests (`resolve`) and into other host services (the working tree via `wasi:filesystem`). This does not change the trait layering — it is handled by passing the backend a host-provided `ToolHost` argument on `eval` (§3.3, §4.2), the same way `WasiKeyValueCtx::open_bucket` is just handed its arguments. The `resolve` callback instantiates the adapter guest fresh and calls its `references` export, which is precisely host-mediated dynamic linking ([guest-registry.md](guest-registry.md) §4) invoked from the host side — so Layer 2 (the tool loop) depends on the registry landing first; Layer 1 does not.

## 3. Layer 1 — The `wasi-model` host core (the boundary)



### 3.1 The WIT

The floor owns a small, generic interface. The guest hands over a complete, self-contained prompt and gets back a validated answer or a typed error — never a transcript, never a model id.

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

The envelope is deliberately generic: `operation` is an opaque string, `answer-schema` is a JSON Schema, and `answer` is a JSON instance. **The floor validates structure, not meaning** — it checks the answer parses and conforms to the schema the prompt carried, and knows nothing of `build` vs `review`. Specify ships the concrete schemas; the floor enforces them. (Whether the typed contract is JSON-Schema-over-strings or generated WIT records is a consumer choice tracked in §7.3 — the floor only needs "a schema it can validate an answer against".)

### 3.2 Prompt / answer / error records (host side)

Mirroring the generated-bindings pattern in `wasi-keyvalue/src/host.rs`, the host crate runs `wasmtime::component::bindgen!` over `wit/` and re-exports the generated `prompt` / `answer` / `error` types. The host-internal representation a backend sees is a thin owned mirror so backends never touch wasmtime types:

```rust
/// What a backend is asked to judge. Pure data — no wasmtime, no model id.
pub struct Prompt {
    pub brief: BriefId,
    pub operation: String,
    pub answer_schema: serde_json::Value,   // parsed JSON Schema
    pub grants: ToolGrants,
}

/// What a backend returns. The `eval` host binding (§3.4) validates `value`
/// against `prompt.answer_schema` before it reaches the guest; the optional
/// `transcript` is the tool-call log a recording wrapper persists (§3.4).
pub struct Answer {
    pub value: serde_json::Value,
    pub transcript: Option<Transcript>,     // tool-call log for record/replay (§3.4)
}
```



### 3.3 The full host scaffold + the `WasiModelCtx` backend trait

`wasi-model` instantiates the shared shape verbatim. The backend trait is `WasiModelCtx` — the direct `WasiKeyValueCtx`/`WasiBlobstoreCtx` analogue, the one place a provider's logic lives — and its method is `eval`. There is **no** bespoke "ModelBackend" trait; the backend *is* the Ctx.

```rust
/// Host-side service for `wasi-model` (the linked-only effect host).
#[derive(Debug)]
pub struct WasiModel;

impl HasData for WasiModel {
    type Data<'a> = WasiModelCtxView<'a>;
}

impl<T> Host<T> for WasiModel
where
    T: WasiModelView + 'static,
{
    fn add_to_linker(linker: &mut Linker<T>) -> anyhow::Result<()> {
        judgment::add_to_linker::<_, Self>(linker, T::model)
    }
}

impl<R> Server<R> for WasiModel where R: Runtime {}   // non-server: default no-op `run`

/// Implemented by `StoreCtx` (the `Linker<T>` type), produced by `omnia_wasi_view!`.
pub trait WasiModelView: Send {
    fn model(&mut self) -> WasiModelCtxView<'_>;
}

/// View into the backend + the resource table — identical to the others.
pub struct WasiModelCtxView<'a> {
    pub ctx: &'a mut dyn WasiModelCtx,
    pub table: &'a mut ResourceTable,
}

/// The backend trait. Implemented by `ModelDefault` (replay, in-tree) and by the
/// model backends in the `backends` repo (`omnia_genai::Client`,
/// `omnia_cursor::Client`). Carries no vendor type. `eval` is handed the prompt and
/// a host-built `ToolHost` (§4.2) — the latter is the only addition over a plain
/// effect Ctx, and it is just an argument, exactly like `open_bucket`'s `identifier`.
pub trait WasiModelCtx: Debug + Send + Sync + 'static {
    fn eval(&self, prompt: Prompt, tools: Arc<dyn ToolHost>) -> FutureResult<Answer>;
}

omnia_wasi_view!(StoreCtx, model);   // the macro, same as every other host
```

Like `WasiKeyValueCtx`, each backend also implements `Backend` (the env-driven `connect` / `ConnectOptions` / `FromEnv` pattern from `crates/omnia/src/traits.rs`), so `runtime!` connects it concurrently at startup. And just as `KeyValueDefault` is the in-memory default, `**ModelDefault` (replay) is the crate's default backend** (§5.4) — so the host crate is complete and testable before any real model exists.

### 3.4 Where validation and record/replay live

Per [RFC-53](rfc-53-wasi-model.md), answer validation and record/replay are *host* concerns, not behaviour each backend re-implements. They land in the two places the standard pattern already offers, so nothing bespoke is introduced:

- **Validation in the** `eval` **host binding.** The generated `eval` binding is implemented on `WasiModel` in `host/model_impl.rs` — the analogue of `wasi-keyvalue/src/host/store_impl.rs`, which is where keyvalue does its `convert_error` / context work. That impl reads `prompt.answer-schema`, calls `ctx.eval(prompt, tools)`, validates the returned `Answer` against the schema, and maps a miss to `error::invalid-answer`. The guest only ever sees a validated answer or a typed error. A backend that runs its own repair loop (genai, [RFC-59](rfc-59-model-tool-loop.md)) consumes validation failures internally and returns only once it passes.
- **Record/replay as composable** `WasiModelCtx` **wrappers.** Because the backend is just a `WasiModelCtx`, a recording backend is a `WasiModelCtx` that wraps another and logs `(prompt, transcript) -> answer`; the replay backend (`ModelDefault`) is a `WasiModelCtx` that serves a recorded answer for an equivalent prompt. No decorator framework — just the existing trait, composed. Both sit at the typed `eval` boundary, so the spawned-agent backend (which owns its own loop) records and replays identically to the in-process genai backend: the recording captures what crossed `eval`, not how the backend produced it.



## 4. Host tool callbacks (`ToolHost`) — lent to the genai backend

The model tool loop — driving a model through tools, accumulating session state, and repairing until the answer validates — is **not** a floor concern. It lives inside the genai backend ([RFC-59](rfc-59-model-tool-loop.md)). The floor's role is to implement the host-side capabilities genai's loop may call and to lend them through a `ToolHost` for one `eval`. The cursor backend and `ModelDefault` ignore `ToolHost`.

### 4.1 The tool surface (genai)

Within one `eval`, the genai backend may expose these tools to the model ([RFC-59](rfc-59-model-tool-loop.md) §"The tool surface"). The floor *implements* them via `ToolHost`; genai *advertises and dispatches* them to the model:

- `**resolve(reference)**` — follow a brief's internal reference. The floor selects the adapter named by `prompt.grants.references`, instantiates it fresh, and calls its exported `references` shelf — host-mediated dynamic linking ([guest-registry.md](guest-registry.md) §4), instance-per-call.
- `**read(path)` / `list(path)**` — inspect the working tree through the capability lent in `prompt.grants.working-tree`. The model sees bounded results, never an OS path or a `descriptor`.
- `**write(path, bytes)**` — accumulate an edit against the session's base tree. Pending edits live in host-held state (`wasi:keyvalue`), not guest memory.
- `**verify(check)**` — request a closed verification profile from `prompt.grants.verify`. The profile definitions and sandboxing are owned by [RFC-60](rfc-60-verify-profiles.md); this design only routes the call.



### 4.2 `ToolHost` — host callbacks the floor lends to the genai backend

The genai backend never touches the registry, the working tree, or kv directly. The floor builds a per-`eval` `ToolHost` and passes it as the `tools` argument to `WasiModelCtx::eval`. Genai turns model tool-calls into these callbacks:

```rust
/// Host-side capabilities for one `eval`, lent to backends that need them
/// (primarily the genai backend). Each method is a typed callback; genai turns
/// model tool-calls into these.
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

The floor constructs the `ToolHost` from the session (§4.4): it captures the registry handle (for `resolve`), the working-tree capability id from `prompt.grants.working-tree`, the kv handle for accumulating `write`s, and the allowed verify profiles.

### 4.3 The repair loop (genai)

The genai backend drives the model until one terminal state ([RFC-59](rfc-59-model-tool-loop.md) §"Repair loop"):

- the answer validates against `prompt.answer_schema` and returns through `eval` (re-gated by the floor's `eval` host binding, §3.4);
- a tool call fails with a typed, non-repairable error → `error::tool-failed`;
- an iteration / token / time / verify budget is exhausted → `error::budget-exhausted`;
- the backend records a failure answer for replay diagnostics.

Invalid candidates are loop inputs, never guest-visible. The loop is budgeted so it fails clearly rather than spinning. Budgets reuse the existing per-call controls (`crates/omnia/src/options.rs`): the enclosing `eval` host call already runs inside the guest's `guest_timeout` and epoch yielding, and `resolve` dispatches inherit the dispatch-depth bound from [guest-registry.md](guest-registry.md) §6.6 — an `eval`→`resolve`→(adapter that calls `eval`) chain is depth-counted like any other host-mediated dispatch.

### 4.4 Session state (genai)

Because guests are instance-per-call, one genai session's durable state — the prompt and expected type, the adapter identity in scope, the base `revision`, the working-tree capability, accumulated edits, and verify results feeding the repair loop — lives in host-held storage, not guest memory ([RFC-59](rfc-59-model-tool-loop.md) §"Session state"). The genai backend keys session state in `wasi:keyvalue` under the prompt hash, so a `write` during one tool turn is visible to a `read` in the next, and a leaked in-memory session is a regression.

## 5. Layer 2 — The backends

All backends implement the *same* `WasiModelCtx` trait (§3.3) and `Backend` (env-driven connect), and are selected purely by what the deployment names in `runtime!` (or, later, the router of [RFC-58](rfc-58-model-backends.md)) — exactly as `KeyValueDefault` vs. `omnia-redis` are swapped for `WasiKeyValue`. The floor's `eval`-binding validation and the composable recording/replay wrappers (§3.4) apply to all of them uniformly, so each backend only has to "produce an answer (with an optional transcript)".

Two shapes of backend fall out of the architecture, and the two we build are one of each:

- **In-process tool loop (genai)** — the genai backend owns the model API, the tool-use loop, session state, and repair semantics ([RFC-59](rfc-59-model-tool-loop.md)). It turns the model's tool calls into `ToolHost` callbacks. The working tree is reached through the *bounded* `read`/`list`/`write` tools (no OS path crosses to the model).
- **Spawned, filesystem-capable agent (cursor)** — the cursor backend spawns an external agent that owns its own loop and reads/writes the working tree directly through the node-local `local-path` it is lent ([architecture.md](architecture.md#the-working-tree), [RFC-55](rfc-55-working-tree.md)). It ignores `ToolHost`. It still returns a validated answer through the same boundary and stays recordable.



### 5.1 Each real backend is an `omnia-<provider>` crate in the `backends` repo

The two real backends follow the `backends`-repo idiom verbatim — the same shape as `omnia-redis` / `omnia-nats`. A provider crate is **host-only** (`#![cfg(not(target_arch = "wasm32"))]`), names its handle `pub struct Client` (`#[derive(Clone)]`), implements `Backend` (`connect_with` + `ConnectOptions`) in `lib.rs`, and implements the WASI-effect Ctx in a per-interface module:

```text
backends/crates/genai/                  # `omnia-genai`
  src/lib.rs                  # pub struct Client; impl Backend for Client; mod config (FromEnv)
  src/model.rs                # impl omnia_wasi_model::WasiModelCtx for Client
backends/crates/cursor/                 # `omnia-cursor`
  src/lib.rs                  # pub struct Client; impl Backend for Client; mod config (FromEnv)
  src/model.rs                # impl omnia_wasi_model::WasiModelCtx for Client
```

`ConnectOptions` uses the `fromenv` derive the other backends use (cf. `omnia-redis`), and the handle is a `#[derive(Clone)]` `Client` exactly like `omnia_redis::Client`:

```rust
#[allow(missing_docs)]
mod config {
    use fromenv::FromEnv;

    #[derive(Debug, Clone, FromEnv)]
    pub struct ConnectOptions {
        #[env(from = "OMNI_MODEL", default = "gpt-5.5")]
        pub model: String,
        // genai:  provider auth (OPENAI_API_KEY / ANTHROPIC_API_KEY / …) is read by `genai` itself.
        // cursor: #[env(from = "CURSOR_API_KEY")] pub api_key: Option<String>,
    }
}
pub use config::ConnectOptions;

impl omnia::FromEnv for ConnectOptions {
    fn from_env() -> Result<Self> {
        Self::from_env().finalize().context("loading connection options")
    }
}
```

Only the default replay backend (`ModelDefault`) lives in the `wasi-model` host crate, in `crates/wasi-model/src/host/default_impl.rs` — the direct `KeyValueDefault` analogue. So the three `eval` implementations are:

```rust
// omnia:    crates/wasi-model/src/host/default_impl.rs   -> ModelDefault (replay, the default)
// backends: crates/genai/src/model.rs                    -> impl WasiModelCtx for Client
// backends: crates/cursor/src/model.rs                   -> impl WasiModelCtx for Client
```



### 5.2 The genai backend — `omnia-genai` (`Client`), frontier / hosted, in-process tool loop

`[genai](https://github.com/jeremychone/rust-genai)` (`genai = "0.6"`) is one ergonomic Rust API over 25+ providers (OpenAI, Anthropic, Gemini, Ollama, …), so "switch frontier / hosted / local providers" is backend config, never a contract change ([RFC-58](rfc-58-model-backends.md)). The model id and provider live entirely here.

**Connect.** `ConnectOptions` reads the model id from env (`OMNI_MODEL`; provider auth like `OPENAI_API_KEY` / `ANTHROPIC_API_KEY` is read by `genai` itself), and `connect_with` builds the underlying `genai::Client` once at startup — the handle is `Client`, like `omnia_redis::Client`:

```rust
// backends/crates/genai/src/lib.rs
#[derive(Clone)]
pub struct Client {
    inner: genai::Client,    // holds provider auth; never crosses the boundary
    model: Arc<str>,         // e.g. "gpt-5.5", "claude-...", "gemini-..."
}

impl Backend for Client {
    type ConnectOptions = ConnectOptions;
    async fn connect_with(opts: ConnectOptions) -> Result<Self> { /* build genai::Client */ }
}
```

`**eval` — the in-process loop.** Map the typed surface onto `genai`'s chat API and run the tool-use loop:

1. Build a `ChatRequest` with a system prompt derived from `prompt.operation`, the brief as a user message, and `with_tools([...])` advertising `resolve` / `read` / `list` / `write` / `verify` as `genai::Tool`s whose schemas mirror the `ToolHost` methods. Set `ChatResponseFormat::JsonSpec(JsonSpec::new(prompt.answer_schema))` so the final answer is structured output validated against the prompt's schema.
2. `client.exec_chat(&self.model, req, None).await`. If `response.into_tool_calls()` is non-empty, dispatch each through the `ToolHost` callback, append the tool responses to the request (genai's `append_tool_use_from_stream_end` / tool-response messages), and loop.
3. When the model returns content instead of tool calls, that content is the candidate JSON answer; genai self-checks it against `prompt.answer_schema` and, on a miss, re-enters the loop with the validation error appended, up to the repair budget. It returns `Answer { value, transcript: Some(log) }`.
4. The `eval` host binding (§3.4) re-validates against the schema as the floor's final gate before the answer reaches the guest, and the optional recording wrapper persists `(prompt, transcript) -> value`.

`resolve` is the interesting tool: dispatching it calls `ToolHost::resolve`, which the floor satisfies by host-mediated dynamic linking into the in-scope adapter's `references` export — a *fresh* adapter instance, isolated from the guest that called `eval`. The model never holds a descriptor or path; `read`/`list`/`write` return bounded results from the lent working-tree capability.

### 5.3 The cursor backend — `omnia-cursor` (`Client`), spawned, filesystem-capable agent

This is the **spawned-agent** backend of [RFC-58](rfc-58-model-backends.md): the native layer spawns a fresh, context-free `cursor-agent` session, hands it the brief, lets it own its own tool loop and read/write the working tree directly through the `local-path` it is lent ([RFC-55](rfc-55-working-tree.md)), and parses a validated answer back. It still returns through the [RFC-53](rfc-53-wasi-model.md) boundary and stays recordable. It is the `omnia-cursor` crate's `Client`, structured exactly like `omnia-genai` (§5.1).

**Why a different shape.** A filesystem-capable agent cannot use the bounded `read`/`list`/`write` tools — it needs real OS paths. That is exactly the `local-path` face of the working tree: an absent `local-path` is a clean capability signal that an agent-driven build is unavailable on this node ([architecture.md](architecture.md#the-working-tree)). So `eval` on this backend checks `prompt.grants.working-tree` resolves to a `local-path`; if not, it returns `error::backend("no local tree on this node")`.

**Connect.** `ConnectOptions` reads `CURSOR_API_KEY` (or relies on a prior `cursor-agent login`) and an optional `OMNI_MODEL`; `connect_with` just validates the `cursor-agent` binary is on `PATH` (no long-lived client to build). No long-lived process — each `eval` spawns a fresh, context-free session (no leaked transcript, per [RFC-58](rfc-58-model-backends.md) risks).

`**eval` — spawn, run to completion, parse.** Using the documented headless surface ([Cursor CLI docs](https://cursor.com/docs/cli/headless)):

```bash
cursor-agent --print --force \
  --output-format json \
  --model "$OMNI_MODEL" \
  --workspace "$LOCAL_PATH" \
  "<brief + 'emit a final JSON answer conforming to this schema: …'>"
```

- `-p/--print` runs non-interactive to completion; `--force` (a.k.a. `--yolo`) grants write access so the agent can edit the working tree in place; `--workspace "$LOCAL_PATH"` scopes it to the lent tree; `--output-format json` emits a single JSON object (with the aggregated final `result`) on success and *no* well-formed object on failure — a clean success/failure signal.
- The backend parses `.result` as the candidate answer. (Operationally, `cursor-agent --print` is known to occasionally hang after completing, so the spawn is wrapped in a wall-clock timeout that maps to `error::budget-exhausted`; the existing per-call `guest_timeout` is the outer bound.)
- The agent's edits land directly in the `local-path` tree; the floor extracts the content-addressed change-set from that tree afterwards (the git-aware `wasi:filesystem` backend, [RFC-55](rfc-55-working-tree.md)), exactly as the `build` lifecycle expects ([architecture.md](architecture.md#lifecycle-of-an-operation)).
- `--output-format stream-json` (line-delimited events with stable session ids) is available if we later want per-tool-event progress in the recorded transcript; the `json` single-object form is enough to start.

The model id, the API key, and the entire agent protocol stay inside this backend; the calling guest sees the same typed `answer` it would from genai or replay.

### 5.4 `ModelDefault` — the default, deterministic (replay) backend

Replay belongs at the `wasi-model` boundary because it is the test substitute for judgment itself ([RFC-53](rfc-53-wasi-model.md)). `ModelDefault` is the crate's **default backend** — the direct `KeyValueDefault` analogue, living in `host/default_impl.rs`: with no API key and no spawned process, it serves the recorded answer for an equivalent prompt and lets one vertical operation run deterministically in CI without a live model.

- **Fixtures** are the `(prompt + transcript) -> validated answer` rows that the recording wrapper (§3.4) writes, keyed by a stable hash of the prompt. The minimal seam here is a directory of JSON fixtures (`OMNI_REPLAY_DIR`); the full fixture management, matching policy, and cross-backend diagnostics are the [RFC-58](rfc-58-model-backends.md) replay *expansion*, out of scope for this slice.
- **Determinism.** Because the record happens at the typed boundary, a fixture captured against `genai` or `cursor-agent` replays identically — CI never depends on which backend produced it.
- A prompt with no matching fixture returns `error::backend("no replay fixture")` (fail loud, never fall through to a live call).



## 6. Proof example (the acceptance vehicle)

A new `examples/model` with one tiny guest, proving the boundary and the loop with no Specify concepts:

- `judge` (guest) — imports `augentic:model/judgment`, builds a `prompt` whose `answer-schema` is a trivial shape (e.g. `{ "verdict": "pass" | "fail", "reason": string }`) plus `tools.references` naming a sibling `shelf` guest, and calls `eval(prompt)`; triggered via CLI.
- `shelf` (guest) — exports a `references` interface; `judge`'s prompt references one entry, so the model's `resolve` lands in a fresh `shelf` instance.

Three acceptance runs over the *same* example, swapping only the bound backend in `runtime!` / config:

1. `**WasiModel: ModelDefault**` — with a checked-in fixture, `eval` returns the validated answer with no network. This is the CI gate (Layer 1).
2. `**WasiModel: omnia_genai::Client**` — against a real provider (gated on a key), the model emits a `resolve` tool call that dispatches into `shelf` (fresh instance, host-mediated linking), then returns a schema-valid answer; the recording wrapper writes the fixture run 1 replays. This proves the boundary + genai backend end-to-end ([RFC-59](rfc-59-model-tool-loop.md)).
3. `**WasiModel: omnia_cursor::Client**` — given a `local-path` working tree, a spawned `cursor-agent` run returns a schema-valid `.result`; with no `local-path` it returns `error::backend`, proving the capability signal. This proves the spawned-agent shape, with no guest or contract change between runs.



## 7. Design decisions and rationale



### 7.1 The floor owns the boundary and validation; the backend owns the model and its loop

The mechanism/population split from [guest-registry.md](guest-registry.md) §6.2 holds here as a mechanism/judgment split. **Omnia owns the mechanism** — the `eval` boundary, the prompt / answer / error envelope, the `WasiModelCtx` backend trait, the `ToolHost` host callbacks (which bind to the registry and working tree for genai's use), schema validation in the `eval` host binding, and the composable recording/replay `WasiModelCtx` wrappers. **The backend owns the judgment** — the model id, the provider SDK or spawned process, the prompt-engineering of the brief, and how it drives the model (including genai's in-process tool loop per [RFC-59](rfc-59-model-tool-loop.md)). The floor compiles knowing zero model ids and zero providers (Law 2).

### 7.2 `resolve` reuses host-mediated dynamic linking — it does not reinvent it

The architecture is explicit that "the `wasi-model` `eval → resolve` callback is this same mechanism applied by the model backend" ([architecture.md](architecture.md#guest-to-guest-interaction-host-mediated-dynamic-linking)). So `ToolHost::resolve` is a thin host-side caller of the [guest-registry.md](guest-registry.md) §4 dispatch: resolve identity → instantiate fresh → invoke the `references` export → return typed bytes. The genai backend invokes it from its tool loop; the floor implements the dispatch. Instance-per-call (no recursive re-entrance) is inherited from the registry rather than re-proven.

### 7.3 The envelope is JSON-Schema-over-strings at the floor; typed records are a consumer opt-in

The floor cannot know Specify's brief / answer *types*, so its generic envelope carries an opaque `answer-schema` (JSON Schema) and validates a JSON `answer` against it — the analogue of the dynamic (`Val`-based) path in [guest-registry.md](guest-registry.md) §6.3. A consumer that owns its types (Specify) may generate WIT records and validate richer typed answers above the floor; both reduce to "an answer the floor can check against a schema". We start with JSON Schema because it is the minimal thing that makes "validated answers only" enforceable generically.

### 7.4 Two backend shapes, deliberately, as the first two

genai and cursor-agent are not arbitrary — they are one of each backend *shape* the architecture names (in-process loop vs. spawned filesystem-capable agent), so building both proves the boundary is general enough for the whole [RFC-58](rfc-58-model-backends.md) catalogue (SLM and router are further in-process-loop and selection variants). The replay backend is built first because it is the only one with no external dependency and it is what keeps CI green throughout.

### 7.5 Vendor churn and keys stay below the boundary

`genai` is pre-1.0 and the `cursor-agent` CLI surface evolves; both are pinned, swappable backend dependencies (the `genai` crate version, a `cursor-agent` version assumption) confined below `WasiModelCtx`, never reaching the `augentic:model` contract or the guests — the same containment discipline [RFC-56](rfc-56-runtime-move.md) applies to wRPC. API keys are read from env inside `connect` and never logged or recorded into fixtures.

## 8. Phased plan

Each phase is independently shippable and keeps `cargo make ci` green.

- **Phase 0 —** `DECISIONS.md` **entries.** Record the settled choices (the mechanism/judgment split; `resolve` reuses host-mediated linking; JSON-Schema-over-strings at the floor; validation in the `eval` binding and record/replay as composable `WasiModelCtx` wrappers; vendor + keys stay below the boundary) into the shared `DECISIONS.md` that [guest-registry.md](guest-registry.md) §7 already seeds.
- **Phase 1 — The** `wasi-model` **host core + replay.** New `crates/wasi-model` built verbatim on the `wasi-keyvalue`/`wasi-blobstore` shape: `wit/model.wit`, the `generated` bindgen! module, the `Prompt`/`Answer` mirrors, the `WasiModel` host struct (`HasData` + `Host` + no-op `Server` on `Runtime`), the `WasiModelView` / `WasiModelCtxView` / `WasiModelCtx` traits, the `omnia_wasi_view!` macro, the `eval` host binding in `host/model_impl.rs` (validation gate), the recording/replay `WasiModelCtx` wrappers, and `ModelDefault` (replay) in `host/default_impl.rs` as the default. `examples/model` run 1 (replay) is the acceptance gate. **No model dependency, no registry dependency** — Layer 1 stands alone. ([RFC-53](rfc-53-wasi-model.md).)
- **Phase 2a — The genai backend (**`omnia-genai`**).** A new `omnia-genai` crate in the `backends` repo (`pub struct Client`, `Backend` + `WasiModelCtx`, `fromenv` `ConnectOptions`), adding `genai = "0.6"` as its own dependency. Implement the floor-side `ToolHost` callbacks in `wasi-model` (`resolve` via the guest registry ([guest-registry.md](guest-registry.md) §4), `read`/`list`/`write` over the lent working-tree capability, session state in `wasi:keyvalue`, the `verify` seam — routing only; profiles are [RFC-60](rfc-60-verify-profiles.md)); map the tool surface onto `ChatRequest` / `Tool` / `JsonSpec`; run genai's in-process loop and repair semantics ([RFC-59](rfc-59-model-tool-loop.md)). `examples/model` run 2 (live + record) is the acceptance gate. **Depends on the guest registry (Phase 1 of guest-registry.md).** ([RFC-58](rfc-58-model-backends.md).)
- **Phase 2b — The cursor backend (**`omnia-cursor`**).** A new `omnia-cursor` crate in the `backends` repo, same shape. Spawn `cursor-agent -p --force --output-format json --workspace`, parse `.result`, wrap in a timeout, enforce the `local-path` capability signal; `examples/model` run 3 is the acceptance gate. ([RFC-58](rfc-58-model-backends.md).)
- **Phase 3 — Hardening.** Replay-fixture expansion (matching policy, diagnostics), `stream-json` transcript capture for richer recordings, the router seam stub, failure-mode tests, docs. (Defers the rest of [RFC-58](rfc-58-model-backends.md) and [RFC-60](rfc-60-verify-profiles.md).)



## 9. Implementation planning approach

As in [guest-registry.md](guest-registry.md) §8, this RFC is the design source of truth, not the execution plan. Turn it into work with the same hybrid: one durable index (the §8 phase list with each phase's exit criteria, dependencies, and the invariants every phase preserves) plus just-in-time per-phase plans.

- **Write Phase 0 + Phase 1 detailed plans now** — they are well-understood, dependency-free, and pure wasmtime + a directory of fixtures.
- **Defer Phase 2a's detailed plan until the guest registry lands**, because `ToolHost::resolve` binds to whatever `GuestRegistry`/dispatch API that work produces — planning it now would be speculation against an unbuilt seam.
- **Defer Phase 2b's detailed plan until Phase 2a**, once the boundary + genai path is concrete.
- **Every plan preserves the invariants** and states how in its acceptance test: instance-per-call through the `resolve` callback; validated-answers-only across `eval`; the floor stays generic (Law 2 — no model id, provider, or Specify schema leaks into Omnia); record/replay works at the boundary for *every* backend; per-call budgets and the dispatch-depth bound hold; `cargo make ci` and all existing examples stay green.



## 10. References

- [architecture.md](architecture.md) — §"Judgment: the `wasi-model` host", §"Resolving references", §"The model backend is swappable", §"The working tree".
- [RFC-53](rfc-53-wasi-model.md) — the `wasi-model` host core: the boundary, backend trait, validation, minimal replay (Layer 1).
- [RFC-59](rfc-59-model-tool-loop.md) — the genai backend's in-process tool loop: `resolve`/`read`/`list`/ `write`/`verify`, session state, repair loop.
- [RFC-58](rfc-58-model-backends.md) — the backend catalogue and router; this RFC builds its frontier (genai) and spawned-agent (cursor-agent) entries plus replay expansion (Layer 2).
- [RFC-56](rfc-56-runtime-move.md) — the runtime move and multi-guest registry that `resolve` dispatches through.
- [guest-registry.md](guest-registry.md) — host-mediated dynamic linking; `resolve` is the same mechanism invoked host-side (§4, §6.2, §6.3).
- [RFC-55](rfc-55-working-tree.md) — the working tree's `descriptor` / `local-path` faces genai's bounded tools and the spawned agent's direct read/write.
- `[genai](https://github.com/jeremychone/rust-genai)` — the frontier-backend dependency: `Client`, `ChatRequest`, `Tool`, `ChatResponseFormat::JsonSpec`, `exec_chat`.
- [Cursor CLI headless docs](https://cursor.com/docs/cli/headless) — the spawned-agent dependency: `--print --force --output-format json --workspace`.

