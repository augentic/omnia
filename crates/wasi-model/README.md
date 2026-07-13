# Omnia WASI Model

This crate provides the `omnia:model/completion` boundary for the Omnia runtime: the domain-agnostic *seam* a guest calls to have a prompt completed (`create: func(request) -> result<reply, error>`).

It owns only the boundary — the provider-shaped `request` (`system` / `messages` / `format` / `tools` / `grants`) and its `reply` / `error` envelope, the `WasiModelCtx` backend trait behind `create`, answer validation (including the JSON-Schema gate for `format::schema`), and the guest-side `Sections` prompt builder. It knows nothing about which model, which provider, or any vendor SDK (Law 2). Real model backends (`omnia-genai`, `omnia-cursor`) live in the `backends` repo behind the same trait; deterministic fixture replay for tests lives in `omnia-testkit`.

## Interface

Implements the `omnia:model` WIT interface (`completion`).

## Backend

- **Default**: `ModelDefault` (echo). It connects with zero configuration and answers every completion with its own prompt — the last message echoed as a string for `format::text`, wrapped as `{"echo": ...}` for `format::json` — so guest wiring runs deterministically with no live model. `format::schema` completions fail loud (no echo can conform to an arbitrary guest schema): bind a real backend, or inject `omnia_testkit::model::ReplayBackend` in tests, which replays recorded answers from JSON fixtures.

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
