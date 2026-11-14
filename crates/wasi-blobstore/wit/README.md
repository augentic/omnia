# WebAssembly Interface Types (WIT)

`blobstore.wit` is `omnia:blobstore`, owned by this crate and edited here. It
mirrors the SDK's `BlobStore` one function per method.

`deps/` holds the `wasi:blobstore` draft the host still serves for raw-binding
guests.

## Prerequisites

Install [wkg](https://github.com/bytecodealliance/wasm-pkg-tools).

## Usage

```bash
wkg get wasi:blobstore@0.2.0-draft --config .wkg-config.toml --output ./crates/wasi-blobstore/wit/deps/wasi-blobstore-0.2.0-draft/package.wit
```
