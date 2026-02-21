# HTTP Server Example

Demonstrates a basic HTTP server using `wasi-http` with GET and POST endpoints.

## Quick Start

```bash
# build the guest
cargo build --example http-wasm --target wasm32-wasip2

# run the host
export RUST_LOG="info,omnia_wasi_http=debug,http=debug"
cargo run --example http -- run ./target/wasm32-wasip2/debug/examples/http_wasm.wasm
```

## Test

```bash
# POST request
curl --header 'Content-Type: application/json' -d '{"text":"hello"}' http://localhost:8080

# GET request
curl http://localhost:8080
```
