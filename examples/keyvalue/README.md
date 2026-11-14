# Key-Value Example

Demonstrates `wasi-keyvalue` using the default (in-memory) implementation.

The guest opens a bucket named `omnia_bucket`. The in-memory backend serves
only the buckets `KEYVALUE_BUCKETS` declares (`default` when unset), so the
host runs with that bucket declared.

## Quick Start

```bash
KEYVALUE_BUCKETS=omnia_bucket make example keyvalue
```

Or, more manually, for debugging:

```bash
# build the guest
cargo build --example keyvalue-wasm --target wasm32-wasip2

# run the host
KEYVALUE_BUCKETS=omnia_bucket cargo run --example keyvalue -- -v run ./target/wasm32-wasip2/debug/examples/keyvalue_wasm.wasm
```

## Test

```bash
curl --header 'Content-Type: application/json' -d '{"text":"hello"}' http://localhost:8080
```
