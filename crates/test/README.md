# omnia-test

Contract rows, a component runtime harness, and a `wasm32-wasip3` fixture
pipeline for code built on omnia.

| Feature | Carries | Depends on |
| ------- | ------- | ---------- |
| `contract` | Behaviour rows every backend of `omnia:keyvalue`, `omnia:blobstore`, `omnia:sql`, or `omnia:docstore` meets, run by a guest component | `omnia-sdk` |
| `host` | `Deployment`, `Backends`, the scripted and recording backends, `send` and `deliver`, `precompiled`, `Scratch` — the component runtime harness | `omnia` and the `wasi-*` host crates |
| `build` | `Components` — the nested wasm32 build and `gen.rs` generator for a `build.rs` | `std` only |

There is no default feature: a consumer names the rung it uses. A handler is
tested as the component it ships in, booted over `Backends` and driven by
`send` or `deliver`. The walk-through is
[Testing Omnia-Based Code](https://github.com/augentic/omnia/blob/main/docs/guides/testing-omnia-code.md).

## Depending on it

Two lines cover a typical guest crate: the harness as a dev dependency, the
fixture pipeline as a build dependency.

```toml
[target.'cfg(not(target_arch = "wasm32"))'.dev-dependencies]
omnia-test = { version = "0.38", features = ["host"] }

[build-dependencies]
omnia-test = { version = "0.38", features = ["build"] }
```

The target gate on the dev line keeps the runtime out of the component's
build. The contract rows run inside a guest, so a backend's guest programs
take `contract` on their `wasm32` dependency line instead. Without
`contract`, the crate is empty on `wasm32`.

## Reviewing the crate

`mise run ci` checks the shapes a consumer sees, not just `--all-features`:

```sh
mise run lint          # clippy --all-features then cargo hack clippy --each-feature, natively and for wasm32-wasip3
mise run test          # cargo nextest run --workspace --all-features
```

The `build` feature must stay `std`-only so a consumer's `build.rs` does not
pull the runtime into its build-dependency graph.
`cargo tree -p omnia-test --no-default-features --features build -e normal`
prints exactly one line.
