# omnia-test

Test doubles, a component runtime harness, and a `wasm32-wasip2` fixture
pipeline for code built on omnia — one crate with additive features, each
naming what a consumer uses.

| Feature | Carries | Depends on |
| ------- | ------- | ---------- |
| `guest` | Native doubles for every `omnia_sdk` capability trait, the `Provider` that bundles them, and the `send`/`deliver` drivers that run one request or one delivery through a guest router | `omnia-sdk` (with `sql`, `docstore`), `tower`, `backend`, `conformance` |
| `host` | `Deployment`, `Backends`, `Scratch` — the component runtime harness | `omnia` and the `wasi-*` host crates, `backend`, `conformance` |
| `backend` | `ScriptedModel`, `ScriptedVcs` — `wasmtime`-free `*Client` doubles | `omnia-backend` |
| `conformance` | One scenario per row of the capability contract, and the `conformance!` macro pairing each with a test | `omnia-sdk` (with `sql`, `docstore`) |
| `build` | `Components` — the nested wasm32 build and `gen.rs` generator for a `build.rs` | `std` only |

There is no default feature: a consumer names the rung it uses. The rungs
share one `Script` core, so a scripted model reads the same at the handler
rung (`guest::ScriptedModel`) and the component rung (`host::ScriptedModel`). The
walk-through is [Testing Omnia-Based Code](https://github.com/augentic/omnia/blob/main/docs/guides/testing-omnia-code.md).

## Depending on it

Two lines cover a typical consumer: the doubles and harness as a dev
dependency, the fixture pipeline as a build dependency.

```toml
[target.'cfg(not(target_arch = "wasm32"))'.dev-dependencies]
omnia-test = { version = "0.38", features = ["guest", "host"] }

[build-dependencies]
omnia-test = { version = "0.38", features = ["build"] }
```

The target gate on the dev line is the canonical shape for a guest crate: its
handler tests compile natively while the component does not see the crate at
all. Only `conformance` compiles on `wasm32`, for the guest that runs the
scenarios over `WasiProvider`; every other module is native, so an ungated
line resolves on both targets and contributes nothing else to the component —
the gate simply keeps the host crates out of the `wasm32` dependency graph.

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
