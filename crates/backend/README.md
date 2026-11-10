# omnia-backend

Engine-free service contracts, resource traits, and value types for Omnia capabilities.

Each capability module splits into two layers:

- **`<cap>.rs`** — target-neutral value types and wire constants, compiled on both `wasm32` and native targets so `omnia-sdk` and backend crates share one definition.
- **`<cap>/client.rs`** — native (`cfg(not(target_arch = "wasm32"))`) service contracts (`*Client`) and resource traits (`Connection`, `Container`, `Locker`, `Endpoint`, `ToolHost`), with zero `wasmtime` dependencies.

The `wasi:keyvalue` contract is the exception: `KeyValueClient`, `Bucket`, and `BUCKET` live in `omnia-wasi-keyvalue`, which builds engine-free with its `host` feature off.

## Backends that name their hosts

The crate root carries what every backend shares, on native targets:

- **`Connect`**, **`FromEnv`**, **`NoOptions`** — how a backend connects at startup.
- **`Capability`** — a host marker type's WIT package name and the slot a deployment holds for it. A `wasi-*` crate declares one beside its client trait with `capability!(WasiFoo, "omnia:foo", FooClient)`.
- **`Serve<T>`** — a capability a backend of type `T` can serve. `capability!` implements it for every `FooClient + Clone` backend.
- **`Backend`** — a backend naming the hosts it serves, as a tuple of one to four marker types.

```rust,ignore
impl omnia_backend::Backend for Redis {
    type Hosts = (WasiKeyValue, WasiMessaging);
}
```

The list form of `omnia::runtime!` reads `Backend::Hosts` to link a backend's hosts and fill their slots.
