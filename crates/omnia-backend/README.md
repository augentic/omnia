# omnia-backend

Engine-free service contracts, resource traits, value types, and in-memory default implementations for Omnia capabilities.

Each capability module splits into three layers:

- **`<cap>.rs`** — target-neutral value types and wire constants, compiled on both `wasm32` and native targets so `omnia-sdk` and backend crates share one definition.
- **`<cap>/client.rs`** — native (`cfg(not(target_arch = "wasm32"))`) service contracts (`*Client`), resource traits, `ToolSession`, and `MountRegistry`, with zero `wasmtime` dependencies.
- **`<cap>/default.rs`** — in-memory and process default implementations (`*Default`), gated behind per-capability `<cap>-default` features (and the `defaults` union feature).
