# Omnia WASI Model

This crate provides the `omnia:model/completion` boundary for the Omnia runtime: the domain-agnostic *seam* a guest calls to have a prompt completed (`create: func(request) -> result<reply, error>`).

It owns only the boundary — the provider-shaped `request` (`system` / `messages` / `format` / `tools` / `grants`) and its `reply` / `error` envelope, the `WasiModelCtx` backend trait behind `create`, answer validation (including the JSON-Schema gate for `format::schema`), the guest-side `Sections` prompt builder, and the composable record / replay `WasiModelCtx` wrappers. It knows nothing about which model, which provider, or any vendor SDK (Law 2). Real model backends (`omnia-genai`, `omnia-cursor`) live in the `backends` repo behind the same trait; only the deterministic replay backend (`ModelDefault`) ships in-tree.

## Interface

Implements the `omnia:model` WIT interface (`completion`).

## Backend

- **Default**: `ModelDefault` (replay). With no API key and no spawned process, it serves the recorded answer for an equivalent prompt from a directory of JSON fixtures (`MODEL_REPLAY_DIR`), so a vertical operation runs deterministically in CI without a live model. A prompt with no matching fixture fails loud.

## Usage

Add this crate to your `Cargo.toml` and use it in your runtime configuration:

```rust,ignore
use omnia::runtime;
use omnia_wasi_model::ModelDefault;

omnia::runtime!({
    hosts: {
        WasiModel: ModelDefault,
    }
});
```

## License

MIT OR Apache-2.0
