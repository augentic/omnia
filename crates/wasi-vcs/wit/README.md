# WebAssembly Interface Types (WIT)

`vcs.wit` is `omnia:vcs`, owned by this crate and edited here.
Every backend serves `types`, `workspace`, and `store`, the `core` world.
`workspaces` and `remotes` are optional, and `imports` adds them to `core`.

## Deps

`deps/` vendors `wasi:filesystem` and `wasi:clocks` at `0.3.2`, for the
`descriptor` a `location` borrows. The host binds them onto
`wasmtime_wasi::p3::bindings`, so a lent root is the `Descriptor` the
runtime already holds. They are `wasmtime-wasi`'s `src/p3/wit/deps`, with
the package versions restamped from `0.3.0` to `0.3.2`.
