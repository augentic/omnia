# Identity Example

Demonstrates `wasi-identity` using the default implementation.

## Quick Start

```bash
make build identity
make run identity
```

Or, more manually, for debugging:

```bash
# build the guest
cargo build --example identity-wasm --target wasm32-wasip2

# configure credentials (copy .env.example to .env first)
set -a && source .env && set +a

# run the host (-v shows startup and readiness; -vv logs each request
# and token fetch)
cargo run --example identity -- -v run ./target/wasm32-wasip2/debug/examples/identity_wasm.wasm
```

## Test

```bash
curl http://localhost:8080
```
