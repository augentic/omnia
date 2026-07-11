# Guest API Example

Demonstrates transport-neutral `Operation` implementations, explicit typed
HTTP route registration, and a small regular-Rust WASI HTTP export adapter.

## Quick Start

```bash
# build the guest
cargo build --example guest-api-wasm --target wasm32-wasip2

# run the host
cargo run --example guest-api -- run ./target/wasm32-wasip2/debug/examples/guest_api_wasm.wasm
```

## Test

```bash
# path parameter
curl http://localhost:8080/greet/omnia

# JSON body, with a correlation header surfaced through invocation metadata
curl --header 'Content-Type: application/json' --header 'X-Request-Id: 42' \
  -d '{"name":"omnia"}' http://localhost:8080/greet
```
