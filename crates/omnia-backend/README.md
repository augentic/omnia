# omnia-backend

Engine-free service contracts, resource traits, value types, and in-memory default implementations for Omnia capabilities.

Value types compile on every target (`wasm32` and native) so `omnia-sdk` and backend crates share one definition. Service traits (`*Client`), resource traits, default implementations (`*Default`), `Connect`, `MountRegistry`, and `Telemetry` compile on native targets (`cfg(not(target_arch = "wasm32"))`) with zero `wasmtime` dependencies.
