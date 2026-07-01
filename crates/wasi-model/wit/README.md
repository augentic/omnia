# WebAssembly Interface Types (WIT)

`model.wit` is the authoritative definition of the `omnia:model@0.1.0` package (the `completion` interface), per `rfcs/wasi-model.md` §3.1.

## Deps

The `deps/` directory vendors the `wasi:filesystem` and `wasi:clocks` packages at version `0.3.0` (p3) — the versions the runtime serves via `wasmtime_wasi::p3::add_to_linker` — so the `grants.workspace` `borrow<descriptor>` resolves, via the host `bindgen!` `with:` remap onto `wasmtime_wasi::p3::bindings`, to the same `Descriptor` resource the runtime already owns. They are copied verbatim from `wasmtime-wasi`'s `src/p3/wit/deps`.

p3 filesystem reads ride native component-model `stream`/`future` rather than `wasi:io` streams, so the p2-only `io.wit` that `filesystem/types` previously imported is no longer vendored (RFC-55).
