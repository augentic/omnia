# Cursor Example

Live model completion via `[omnia-cursor](https://crates.io/crates/omnia-cursor)`: the `ask` guest calls `create` once (command mode) while an HTTP server serves `/ask` and the `[mcp](../mcp)` docs guest at `/mcp/docs` for the spawned `cursor-agent`.

Requires the `[backends](https://github.com/augentic/backends)` checkout beside this repo and `[cursor-agent](https://cursor.com/docs/cli)` on `PATH` (`cursor-agent login`).

## Build and run

```bash
cargo build -p examples --example cursor-wasm --example mcp-wasm --target wasm32-wasip2

cargo run --example cursor -- run --config examples/cursor/omnia.toml
```

The runtime defaults `CURSOR_MCP_SERVERS` to `{"docs":"http://localhost:8080/mcp/docs"}` so the guest's `docs` MCP grant resolves to the sibling docs guest, and creates a temp workspace when `OMNIA_WORKSPACE` is unset. Override either variable when needed.

## Test

The run command above includes a test that calls `create` once and prints the answer.

## MCP servers

MCP wiring is opt-in per completion and spans three layers:

1. **Prompt grant** — the guest names a logical server in `tools` (here, `docs` in `guest.rs`). Only granted names are wired into the spawned `cursor-agent`.
2. **Deployment map** — `CURSOR_MCP_SERVERS` is a JSON object mapping each logical name to an HTTP endpoint, e.g. `{"docs":"http://localhost:8080/mcp/docs"}`. The example runtime sets this default when the variable is unset; override it to point at other MCP servers.
3. **HTTP route** — `omnia.toml` serves the endpoint as a WASM guest (`/mcp/docs` → the `[mcp](../mcp)` docs server).

When a completion runs, `omnia-cursor` resolves the grant against `CURSOR_MCP_SERVERS`, merges the matching entries into `<workspace>/.cursor/mcp.json` for the spawn, and passes `--approve-mcps`. `cursor-agent` has no `--mcp-config` flag; it discovers servers only from that file (or `~/.cursor/mcp.json`).