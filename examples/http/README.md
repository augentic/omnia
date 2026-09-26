# HTTP Server Example

Demonstrates a basic HTTP server using `wasi-http` with GET and POST endpoints.

## Quick Start

```bash
make build http
make run http
```

Or, more manually, for debugging:

```bash
# build the guest
cargo build --example http-wasm --target wasm32-wasip2

# run the host (-v shows startup and readiness; -vv logs each request)
cargo run --example http -- -v run ./target/wasm32-wasip2/debug/examples/http_wasm.wasm
```

## Test

```bash
# POST request
curl --header 'Content-Type: application/json' -d '{"text":"hello"}' http://localhost:8080

# GET request
curl http://localhost:8080
```
