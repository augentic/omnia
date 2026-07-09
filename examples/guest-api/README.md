# Guest API Example

Demonstrates the `guest!` macro and the typed `Handler` API: a declarative
route table generates the WASI HTTP export and axum router, and each request
type parses its own input and produces a `Reply`.

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

# JSON body, with a header surfaced through `Context::headers`
curl --header 'Content-Type: application/json' --header 'X-Request-Id: 42' \
  -d '{"name":"omnia"}' http://localhost:8080/greet
```
