# WebAssembly Interface Types (WIT)

`model.wit` is `omnia:model`, owned by this crate and edited here.
Every backend serves `types` and `completion`, the `core` world.
`mcp` is optional, and `imports` adds it.

## Deps

`deps/` vendors `wasi:filesystem` and `wasi:clocks` at `0.3.2`, for the
`descriptor` a `location` borrows. The host binds them onto
`wasmtime_wasi::p3::bindings`, so a lent root is the `Descriptor` the
runtime already holds. They are copied verbatim from `wasmtime-wasi`'s
`src/p3/wit/deps`.
