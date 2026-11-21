# Config Example

Demonstrates listing runtime configuration over `omnia:config`.

The runtime serves the host variables `HOME` and `USER`, the keys it
compiles into `ConfigDefault`. A runtime that compiles none in serves the
variables `CONFIG_KEYS` names, and none when it is unset.

## Quick Start

```bash
make example config
```

Or, more manually, for debugging:

```bash
# build the guest
cargo build --example config-wasm --target wasm32-wasip3

# run the host
cargo run --example config -- -v run ./target/wasm32-wasip3/debug/examples/config_wasm.wasm
```

## Test

```bash
curl http://localhost:8080
```
