# Blobstore Example

Demonstrates `wasi-blobstore` using the default (in-memory) implementation.

## Quick Start

```bash
make build blobstore
make run blobstore
```

Or, more manually, for debugging:

```bash
# build the guest
cargo build --example blobstore-wasm --target wasm32-wasip2

# run the host
cargo run --example blobstore -- -v run ./target/wasm32-wasip2/debug/examples/blobstore_wasm.wasm
```

## Test

```bash
curl --header 'Content-Type: application/json' -d '{"text":"hello"}' http://localhost:8080
```
