# Model Completions and MCP

The `omnia:model/completion` interface lets a sandboxed guest request a completion from a large language model without knowing which model, provider, or agent serves it. The guest states *what* it wants (prompt, response format, capability grants); the host validates the request, injects tools, runs the backend, and validates the answer before the guest sees it.

This page covers the guest API, the grants model, the available backends, and how guests can also *serve* tools to models over MCP (the [Model Context Protocol](https://modelcontextprotocol.io)).

## Requesting a completion from a guest

A model guest is typically a command-mode guest (see [Writing Guests](writing-guests.md#command-mode-guests)). It builds a `Request` and calls `completion::create`:

```rust,noplayground
let (system, messages) = Sections {
    role: Some("a terse code reviewer".to_string()),
    task: "decide whether the change is acceptable".to_string(),
    context: Some("the diff adds a bounds check".to_string()),
    ..Sections::default()
}
.channels(None);

let request = completion::Request {
    model: None,
    system,
    messages,
    generation: None,
    format: completion::Format::Schema(completion::Schema {
        name: "verdict".to_string(),
        schema: "{\"type\":\"object\"}".to_string(),
    }),
    tools: vec![],
    grants: completion::Grants {
        references: Some("shelf".to_string()),
        workspace,
        verify: vec![],
    },
};

let answer = match completion::create(request).await {
    Ok(reply) => reply.answer,
    Err(error) => format!("error: {error:?}"),
};
```

The pieces:

- **`Sections`** — a guest-side builder that assembles the `system` and `messages` channels from structured fields (role, task, context, ...), so prompts stay consistent across guests.
- **`model: None`** — which concrete model serves the request is deployment configuration; the guest can suggest a model id but usually leaves it to the backend.
- **`format`** — `Text`, or `Schema` for a JSON answer validated against a JSON Schema. The host enforces this: an answer that fails validation never reaches the guest.
- **`tools`** — functions the guest itself declares for the model to call, or MCP servers to attach (backend-dependent; see below).
- **`grants`** — capabilities the guest lends to the completion.

## Grants and host-injected tools

Grants are the security boundary. Rather than giving the model backend ambient access, the guest explicitly lends:

- **`workspace`** — a directory descriptor from the guest's own preopen table (populated by the host's `[[mount]]`; see [Multi-Guest Deployments](multi-guest-deployments.md#mounts-giving-guests-a-workspace)). The model can only see a tree the host mounted *and* the guest chose to lend.
- **`references`** — an identifier the host resolves to reference material through guest dispatch.
- **`verify`** — named verification profiles the model may run.

From these grants the **host** — not the guest, not the backend — merges the injected tools `resolve`, `read`, `list`, `write`, and `verify` into the completion. Guests must not redeclare those names in `tools`. Backends receive a `ToolHost` handle and call back into the host to execute them, so every tool invocation passes through the host's validation gate.

## Backends

### `ModelDefault` — deterministic echo (in-tree)

The default backend connects with zero configuration and answers every completion with its own prompt: the last message echoed as a string for `format::text`, wrapped as `{"echo": ...}` for `format::json`. That makes guest wiring and prompt assembly smoke-testable with no live model. `format::schema` requests fail with a `backend` error — no echo can conform to an arbitrary guest schema — so bind a real backend for typed answers.

For tests, CI, and local development of model guests, inject `omnia_testkit::model::ReplayBackend`: it replays recorded answers from JSON fixtures — no network, no credentials, fully deterministic. The [`model` example](../../examples/model/) embeds its checked-in fixture and serves it through `ReplayBackend`:

```bash
cargo build --example model-wasm --target wasm32-wasip2
cargo run --example model -- run --config examples/model/omnia.toml
```

### `omnia-genai` — provider APIs (backends repo)

Calls LLM provider APIs in-process via the [`genai`](https://crates.io/crates/genai) SDK (OpenAI, Anthropic, Gemini, Groq, Ollama, and others). Provider API keys are read from the environment at call time (`OPENAI_API_KEY`, `ANTHROPIC_API_KEY`, ...). It runs a bounded tool loop for the host-injected `resolve` tool and passes guest-declared functions through to the provider. MCP tools are not supported by this backend — use `omnia-cursor` for that.

### `omnia-cursor` — cursor-agent (backends repo)

Spawns the [`cursor-agent`](https://cursor.com/docs/cli) CLI per completion, giving the model a full agentic session inside the granted workspace:

- Requires `cursor-agent` on `PATH` and authentication (`CURSOR_API_KEY` or a prior `cursor-agent login`).
- The workspace grant is mandatory: the agent runs in the directory behind the guest's `grants.workspace` mount.
- `Tool::Mcp` grants are honoured by writing the server URLs into the workspace's `.cursor/mcp.json` for the session (restored afterwards).

Wire it like any other backend:

```rust
use omnia_cursor::Client as Cursor;

omnia::runtime!({
    mode: command,
    hosts: {
        WasiHttp: HttpDefault,
        WasiOtel: OtelDefault,
        WasiModel: Cursor,
    }
});
```

The end-to-end demo lives at [`backends/examples/cursor`](https://github.com/augentic/backends/tree/main/examples/cursor).

## Serving MCP tools from a guest

Guests can also sit on the other side of the protocol: exposing tools and resources to model backends as a stateless MCP server over HTTP. Implement `omnia_guest::mcp::McpServer` and serve `mcp::router` from the guest's HTTP handler:

```rust,noplayground
struct HttpGuest;
wasip3::http::service::export!(HttpGuest);

impl Guest for HttpGuest {
    async fn handle(request: Request) -> Result<Response, ErrorCode> {
        omnia_wasi_http::serve(mcp::router(Docs), request).await
    }
}
```

The `McpServer` trait has five methods: `info` (server identity), `tools` (tool declarations with JSON Schema inputs), `call_tool`, and optionally `resources`/`read_resource`. The router handles the JSON-RPC and Streamable HTTP transport details.

The [`mcp`](../../examples/mcp/) example serves a small document set; combined with the cursor backend and an MCP grant, a completion can call back into guest-served tools — a guest-to-model-to-guest loop entirely under host mediation.
