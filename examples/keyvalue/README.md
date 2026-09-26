# Key-Value Example

Demonstrates `wasi-keyvalue` using the default (in-memory) implementation.

## Quick Start

```bash
make build keyvalue
make run keyvalue
```

Or, more manually, for debugging:

```bash
# build the guest
cargo build --example keyvalue-wasm --target wasm32-wasip2

# run the host (-v shows startup and readiness; -vv logs each request;
# RUST_LOG=omnia_wasi_keyvalue=trace traces every store operation)
cargo run --example keyvalue -- -v run ./target/wasm32-wasip2/debug/examples/keyvalue_wasm.wasm
```

## Test

```bash
curl --header 'Content-Type: application/json' -d '{"text":"hello"}' http://localhost:8080
```
