# MCP Documentation Server Example

A wasm guest that serves compiled-in markdown to agent backends as a stateless
[Model Context Protocol](https://modelcontextprotocol.io) server over
`wasi:http`. It exposes `list_docs` and `read_doc` tools (and matching `doc://`
resources).

## Build and run

```bash
cargo build --example mcp-wasm --target wasm32-wasip2
cargo run --example mcp -- run --config examples/mcp/omnia.toml
```

Endpoint: `http://localhost:8080/mcp/docs`

## Test

```bash
U=http://localhost:8080/mcp/docs
curl -s -X POST "$U" -H 'content-type: application/json' \
  -d '{"jsonrpc":"2.0","id":1,"method":"tools/list"}'
curl -s -X POST "$U" -H 'content-type: application/json' \
  -d '{"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":"read_doc","arguments":{"name":"overview"}}}'
```

With `cursor-agent`, point a scratch workspace at the server via
`.cursor/mcp.json` (`{"mcpServers":{"omnia-docs":{"url":"http://localhost:8080/mcp/docs"}}}`)
and ask it to use the tools — for example, to report the allowed widget state
transitions.

The sample corpus (`overview`, `api-reference`, `style-guide`) lives in
[`guest.rs`](guest.rs).
