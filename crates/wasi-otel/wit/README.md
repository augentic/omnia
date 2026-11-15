# WebAssembly Interface Types (WIT)

`otel.wit` is the `omnia:otel@0.2.0` package.

## Prerequisites

Install [wkg](https://github.com/bytecodealliance/wasm-pkg-tools).

## Usage

```bash
wkg get omnia:otel@0.2.0 --config .wkg-config.toml --output ./crates/wasi-otel/wit/otel.wit
```

## Deps

`deps/clocks.wit` vendors the `wasi:clocks` package at version `0.3.2`, for
the `instant` every timestamp carries. The host binds it onto
`wasmtime_wasi::p3::bindings::clocks`, the `0.3` track the runtime serves. It
is copied verbatim from `wasmtime-wasi`'s `src/p3/wit/deps`, as in
`wasi-model` and `wasi-vcs`.
