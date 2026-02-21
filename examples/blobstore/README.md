# Blobstore Example

Demonstrates `wasi-blobstore` using the default (in-memory) implementation.

## Quick Start

```bash
# build the guest
cargo build --example blobstore-wasm --target wasm32-wasip2

# run the host
export RUST_LOG="info,wasi_blobstore=debug,omnia_wasi_http=debug,blobstore=debug"
cargo run --example blobstore -- run ./target/wasm32-wasip2/debug/examples/blobstore_wasm.wasm
```

## Test

```bash
curl --header 'Content-Type: application/json' -d '{"text":"hello"}' http://localhost:8080
```
