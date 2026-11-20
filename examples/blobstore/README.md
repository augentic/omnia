# Blobstore Example

Demonstrates `omnia:blobstore`, through the `omnia_sdk::blobstore` bindings and
their `Container` helpers, using the default (in-memory) implementation.

## Quick Start

```bash
make example blobstore
```

Or, more manually, for debugging:

```bash
# build the guest
cargo build --example blobstore-wasm --target wasm32-wasip3

# run the host
cargo run --example blobstore -- -v run ./target/wasm32-wasip3/debug/examples/blobstore_wasm.wasm
```

## Test

```bash
curl --header 'Content-Type: application/json' -d '{"text":"hello"}' http://localhost:8080
```
