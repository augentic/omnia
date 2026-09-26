# WebSocket Server Example

Demonstrates `wasi-websocket` for real-time bidirectional communication.

## Quick Start

```bash
make build websocket
make run websocket
```

Or, more manually, for debugging:

```bash
# build the guest
cargo build --example websocket-wasm --target wasm32-wasip2

# run the host
cargo run --example websocket -- -v run ./target/wasm32-wasip2/debug/examples/websocket_wasm.wasm
```

## Test

```bash
curl --header 'Content-Type: application/json' -d '{"text":"hello"}' http://localhost:8080
```
