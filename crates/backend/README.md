# omnia-backend

Engine-free service contracts, resource traits, and value types for Omnia capabilities.

Each capability module splits into two layers:

- **`<cap>.rs`** — target-neutral value types and wire constants, compiled on both `wasm32` and native targets so `omnia-sdk` and backend crates share one definition.
- **`<cap>/client.rs`** — native (`cfg(not(target_arch = "wasm32"))`) service contracts (`*Client`) and resource traits (`Bucket`, `Connection`, `Container`, `Locker`, `Endpoint`, `ToolHost`), with zero `wasmtime` dependencies.
