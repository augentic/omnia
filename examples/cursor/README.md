# Cursor agent (live model backend)

End-to-end demo of the [`omnia-cursor`](https://crates.io/crates/omnia-cursor) backend: the host binds `WasiModel` to the spawned-`cursor-agent` implementation and serves two wasm guests under one HTTP server:

- `/ask` — a guest that calls `complete` once (asks for the widget lifecycle).
- `/mcp/docs` — the [`mcp`](../mcp) example guest, a read-only MCP documentation server. With `CURSOR_MCP_URL` set, the spawned `cursor-agent` answers the `/ask` question by calling this server's `list_docs`/`read_doc` tools.

Unlike most examples in this directory, this one requires the [`backends`](https://github.com/augentic/backends) checkout beside this repo (for `omnia-cursor`) and the [`cursor-agent`](https://cursor.com/docs/cli) CLI on `PATH`.

## Build

```bash
cargo build -p examples --example cursor-wasm --example mcp-wasm --target wasm32-wasip2
```

## Run

`cursor-agent` must be on `PATH` and authenticated (`cursor-agent login`).

```bash
export CURSOR_MCP_URL=http://localhost:8080/mcp/docs
export OMNIA_WORKSPACE=$(mktemp -d)
cargo run --example cursor -- run --config examples/cursor/omnia.toml
```

## Test

```bash
curl -s http://localhost:8080/ask
```

The host writes a scratch `.cursor/mcp.json` advertising `CURSOR_MCP_URL` and spawns `cursor-agent` with `--approve-mcps`; the agent reads the docs over MCP and returns the widget stages (`draft`, `assembled`, `shipped`).
