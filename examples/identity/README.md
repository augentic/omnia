# Identity Example

Demonstrates `wasi-identity` using the default implementation.

## Quick Start

```bash
# build the guest
cargo build --example identity-wasm --target wasm32-wasip2

# configure credentials and logging (copy .env.example to .env first)
set -a && source .env && set +a
cargo run --example identity -- run ./target/wasm32-wasip2/debug/examples/identity_wasm.wasm
```

The `.env` file sets `RUST_LOG` (see [`.env.example`](.env.example)) so the host prints startup lines such as `initializing runtime` and `http server listening on: …`.

## Test

```bash
curl http://localhost:8080
```
