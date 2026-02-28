# WebAssembly Interface Types (WIT) Deps

## Prerequisites

Install [wkg](https://github.com/bytecodealliance/wasm-pkg-tools).

## Usage

```bash
wkg get omnia:otel@0.1.0 --config .wkg-config.toml --output ./crates/wasi-otel/wit/otel.wit
wkg wit fetch --config .wkg-config.toml --wit-dir ./crates/wasi-otel/wit
```
