# MCP Documentation Server Example

A wasm guest that serves a few compiled-in documents to agent backends as a
stateless [Model Context Protocol](https://modelcontextprotocol.io) server over
the `wasi:http` trigger. It exposes `list_docs` and `read_doc` tools (and
matching `doc://` resources).

## Build and run

```bash
cargo build --example mcp-wasm --target wasm32-wasip2
cargo run --example mcp -- run --config examples/mcp/omnia.toml
```

The MCP endpoint is `http://localhost:8080/mcp/docs`.

## Test with curl

```bash
U=http://localhost:8080/mcp/docs
curl -s -X POST "$U" -H 'content-type: application/json' \
  -d '{"jsonrpc":"2.0","id":1,"method":"tools/list"}'
curl -s -X POST "$U" -H 'content-type: application/json' \
  -d '{"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":"read_doc","arguments":{"name":"overview"}}}'
```

## Test with cursor-agent

`cursor-agent` discovers MCP servers from `.cursor/mcp.json`, not a flag. Point
it at the running server from a scratch workspace:

```bash
mkdir -p /tmp/mcp-demo/.cursor
echo '{"mcpServers":{"omnia-docs":{"url":"http://localhost:8080/mcp/docs"}}}' \
  > /tmp/mcp-demo/.cursor/mcp.json

cursor-agent --workspace /tmp/mcp-demo --print --approve-mcps --trust \
  "Use the omnia-docs tools to report the allowed widget state transitions."
```
