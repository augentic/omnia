# WebAssembly Interface Types (WIT)

`vcs.wit` is the authoritative definition of the `omnia:vcs@0.1.0` package and its `store`, `workspace`, and `transport` interfaces.

Every operation is an `async func`, like every backend-touching WASI function in this workspace: the host binding runs in wasmtime's concurrent (`store`) mode and awaits the async backend (each `WasiVcsCtx` method returns a future) without blocking the executor. A version-control backend spawns a process per operation, so even the lookups (`resolve`, `head`, `label`) are asynchronous.

A `location` is the `workspace-grant` idiom of `omnia:model`: a borrowed mount-root descriptor plus a plain relative subpath (`/`-separated, no empty, `.`, or `..` segment; empty for the mount itself). The host resolves it against the mount registry, so a backend works only beneath a mount the deployment authorised, and a location that does not exist yet (`init`, `add`, `clone` create one) resolves through its deepest existing ancestor beneath the mount.

## Deps

The `deps/` directory vendors the `wasi:filesystem` and `wasi:clocks` packages at version `0.3.0` (p3) — the versions the runtime serves via `wasmtime_wasi::p3::add_to_linker` — so the `location` `borrow<descriptor>` resolves, via the host `bindgen!` `with:` remap onto `wasmtime_wasi::p3::bindings`, to the same `Descriptor` resource the runtime already owns. They are copied verbatim from `wasmtime-wasi`'s `src/p3/wit/deps`, as `wasi-model` vendors them.
