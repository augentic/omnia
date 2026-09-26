# Vault Example

Demonstrates `wasi-vault` using the default (in-memory) implementation for secure secret storage.

## Quick Start

```bash
make build vault
make run vault
```

Or, more manually, for debugging:

```bash
# build the guest
cargo build --example vault-wasm --target wasm32-wasip2

# run the host
cargo run --example vault -- -v run ./target/wasm32-wasip2/debug/examples/vault_wasm.wasm
```

## Test

```bash
curl --header 'Content-Type: application/json' -d '{"text":"hello"}' http://localhost:8080
```
