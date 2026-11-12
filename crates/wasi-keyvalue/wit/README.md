# WebAssembly Interface Types (WIT)

`keyvalue.wit` is `omnia:keyvalue`, owned by this crate and edited here. It
mirrors the SDK's `StateStore` one function per method.

`deps/` holds the `wasi:keyvalue` draft the host still serves for raw-binding
guests.

## Prerequisites

Install [wkg](https://github.com/bytecodealliance/wasm-pkg-tools).

## Usage

```bash
wkg get wasi:keyvalue@0.2.0-draft2 --config .wkg-config.toml --output ./crates/wasi-keyvalue/wit/deps/wasi-keyvalue-0.2.0-draft2/package.wit
```
