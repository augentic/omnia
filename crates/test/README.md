# omnia-test

Test doubles, capability contract rows, a component runtime harness, and a
`wasm32-wasip2` fixture pipeline for code built on omnia.

| Feature | Carries | Depends on |
| ------- | ------- | ---------- |
| `guest` | Native doubles for every `omnia_sdk` capability trait and the `Provider` that bundles them | `omnia-sdk` |
| `contract` | Behaviour rows every provider of `StateStore`, `SqlDb`, or `DocumentDb` meets, natively and on `wasm32` | `omnia-sdk` |
| `host` | `Deployment`, `Backends`, `ScriptedModel`, `Scratch` — the component runtime harness | `omnia` and the `wasi-*` host crates |
| `build` | `Components` — the nested wasm32 build and `gen.rs` generator for a `build.rs` | `std` only |

There is no default feature: a consumer names the rung it uses. `guest` and
`host` share one `Script` core, so a scripted model reads the same at the
handler rung (`guest::ScriptedModel`) and the component rung
(`host::ScriptedModel`). The walk-through is
[Testing Omnia-Based Code](https://github.com/augentic/omnia/blob/main/docs/guides/testing-omnia-code.md).

## Depending on it

Two lines cover a typical consumer: the doubles and harness as a dev
dependency, the fixture pipeline as a build dependency.

```toml
[target.'cfg(not(target_arch = "wasm32"))'.dev-dependencies]
omnia-test = { version = "0.36", features = ["guest", "host"] }

[build-dependencies]
omnia-test = { version = "0.36", features = ["build"] }
```

The target gate on the dev line is the canonical shape for a guest crate: its
handler tests compile natively while the component does not see the crate at
all. Without `contract`, the crate is also empty on `wasm32`, so an ungated
line resolves on both targets and contributes nothing to the component.

## Reviewing the crate

`mise run ci` checks the shapes a consumer sees, not just `--all-features`:

```sh
mise run lint          # clippy --all-features then cargo hack clippy --each-feature, natively and for wasm32-wasip2
mise run test          # cargo nextest run --workspace --all-features
```

The `build` feature must stay `std`-only so a consumer's `build.rs` does not
pull the runtime into its build-dependency graph.
`cargo tree -p omnia-test --no-default-features --features build -e normal`
prints exactly one line.
