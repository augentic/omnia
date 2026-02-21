# Vault Example

Demonstrates `wasi-vault` using the default (in-memory) implementation for secure secret storage.

## Quick Start

```bash
# build the guest
cargo build --example vault-wasm --target wasm32-wasip2

# run the host
export RUST_LOG="info,wasi_vault=debug,omnia_wasi_http=debug,vault=debug"
cargo run --example vault -- run ./target/wasm32-wasip2/debug/examples/vault_wasm.wasm
```

## Test

```bash
curl --header 'Content-Type: application/json' -d '{"text":"hello"}' http://localhost:8080
```
