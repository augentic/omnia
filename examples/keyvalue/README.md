# Key-Value Example

Demonstrates `wasi-keyvalue` using the default (in-memory) implementation.

## Quick Start

```bash
# build the guest
cargo build --example keyvalue-wasm --target wasm32-wasip2

# run the host
export RUST_LOG="info,wasi_keyvalue=debug,omnia_wasi_http=debug,keyvalue=debug"
cargo run --example keyvalue -- run ./target/wasm32-wasip2/debug/examples/keyvalue_wasm.wasm
```

## Test

```bash
curl --header 'Content-Type: application/json' -d '{"text":"hello"}' http://localhost:8080
```
