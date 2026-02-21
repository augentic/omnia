# Config Example

Demonstrates a basic Config using `wasi-config`.

## Quick Start

```bash
# build the guest
cargo build --example config-wasm --target wasm32-wasip2

# run the host
export RUST_LOG="info,omnia_wasi_http=debug,http=debug"
cargo run --example config -- run ./target/wasm32-wasip2/debug/examples/config_wasm.wasm
```

## Test

```bash
curl http://localhost:8080
```
