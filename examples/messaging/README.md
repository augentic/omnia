# Messaging Example

Demonstrates `wasi-messaging` using the default (in-memory) implementation for pub-sub messaging.

## Quick Start

```bash
make build messaging
make run messaging
```

Or, more manually, for debugging:

```bash
# build the guest
cargo build --example messaging-wasm --target wasm32-wasip2

# run the host
export RUST_LOG="info,opentelemetry_sdk=off,wasi_messaging=debug,messaging=debug"
cargo run --example messaging -- run ./target/wasm32-wasip2/debug/examples/messaging_wasm.wasm
```

## Test

```bash
curl --header 'Content-Type: application/json' -d '{"text":"hello"}' http://localhost:8080/pub-sub
```
