# WebAssembly Interface Types (WIT)

`model.wit` is the authoritative definition of the `augentic:model@0.1.0`
package (the `completion` interface), per `rfcs/wasi-model.md` §3.1.

## Deps

The `deps/` directory vendors the `wasi:filesystem`, `wasi:clocks`, and
`wasi:io` packages at version `0.2.12` — the versions `wasmtime-wasi` 46 ships —
so the `grants.working-tree` `borrow<descriptor>` resolves, via the host
`bindgen!` `with:` remap, to the same `Descriptor` resource the runtime already
owns. They are copied verbatim from `wasmtime-wasi`'s `src/p2/wit/deps`.
