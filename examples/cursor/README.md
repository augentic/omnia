# Cursor Example

Live model completion via `[omnia-cursor](https://crates.io/crates/omnia-cursor)`: the `ask` guest calls `complete` once (command mode) while an HTTP server serves `/ask` and the `[mcp](../mcp)` docs guest at `/mcp/docs` for the spawned `cursor-agent`.

Requires the `[backends](https://github.com/augentic/backends)` checkout beside this repo and `[cursor-agent](https://cursor.com/docs/cli)` on `PATH` (`cursor-agent login`).

## Build and run

```bash
cargo build -p examples --example cursor-wasm --example mcp-wasm --target wasm32-wasip2

export CURSOR_MCP_URL=http://localhost:8080/mcp/docs
export OMNIA_WORKSPACE=$(mktemp -d)
cargo run --example cursor -- run --config examples/cursor/omnia.toml
```

## Test

The run command above includes a test that calls `complete` once and prints the answer.

