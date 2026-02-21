# WebSocket Server Example

Demonstrates `wasi-websocket` for real-time bidirectional communication.

## Quick Start

```bash
# build the guest
cargo build --example websocket-wasm --target wasm32-wasip2

# run the host
export RUST_LOG="info,wasi_websocket=debug,omnia_wasi_http=debug,websocket=debug"
cargo run --example websocket -- run ./target/wasm32-wasip2/debug/examples/websocket_wasm.wasm
```

## Test

```bash
curl --header 'Content-Type: application/json' -d '{"text":"hello"}' http://localhost:8080
```
