# MCP Server Example

A wasm guest that serves markdown to agent backends as a stateless [Model Context Protocol](https://modelcontextprotocol.io) server. It exposes `list_docs` and `read_doc` tools (and matching `doc://` resources).

An end-to-end example using `WasiModel` and `cursor-agent` is available in the `cursor` [example](https://github.com/augentic/backends/tree/main/examples/cursor).

## Quick Start

```bash
make build mcp
make run mcp
```

Or, more manually, for debugging:

```bash
# build the guest
cargo build --example mcp-wasm --target wasm32-wasip2

# run the host
export RUST_LOG="info,opentelemetry_sdk=off,omnia_wasi_http=debug,mcp=debug"
cargo run --example mcp -- run ./target/wasm32-wasip2/debug/examples/mcp_wasm.wasm
```

## Test

List tools:

```bash
curl -s -X POST http://localhost:8080/mcp/docs \
  -H 'content-type: application/json' \
  -d '{"jsonrpc":"2.0","id":1,"method":"tools/list"}'
```

Tool call:

```bash
curl -s -X POST http://localhost:8080/mcp/docs \
-H 'content-type: application/json' \
  -d '{"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":"read_doc","arguments":{"name":"overview"}}}'
```

