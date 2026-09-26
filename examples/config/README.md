# Config Example

Demonstrates a basic Config using `wasi-config`.

## Quick Start

```bash
make build config
make run config
```

Or, more manually, for debugging:

```bash
# build the guest
cargo build --example config-wasm --target wasm32-wasip2

# run the host (-v shows startup and readiness; -vv logs each request)
cargo run --example config -- -v run ./target/wasm32-wasip2/debug/examples/config_wasm.wasm
```

## Test

```bash
curl http://localhost:8080
```
