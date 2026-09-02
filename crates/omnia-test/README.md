# omnia-test

Test doubles, a component runtime harness, and a `wasm32-wasip2` fixture
pipeline for code built on omnia — one native-only crate with three additive
features.

| Feature | Carries | Depends on |
| ------- | ------- | ---------- |
| `guest` (default) | Native doubles for every `omnia_guest` capability trait, the `doubles!` and `forward!` macros | `omnia-guest` |
| `host` | `Deployment`, `Backends`, `ScriptedModel`, `Scratch` — the component runtime harness | `omnia` and the `wasi-*` host crates |
| `build` | `Components` — the nested wasm32 build and `gen.rs` generator for a `build.rs` | `std` only |

All three share one `Script` core, so a scripted model reads the same at the
handler rung (`guest::Scripted`) and the component rung (`host::ScriptedModel`).

## Depending on it

Two lines cover a typical consumer: the doubles and harness as a dev
dependency, the fixture pipeline as a build dependency.

```toml
[dev-dependencies]
omnia-test = { version = "0.36", features = ["guest", "host"] }

[build-dependencies]
omnia-test = { version = "0.36", default-features = false, features = ["build"] }
```

The crate is empty on `wasm32` (`#![cfg(not(target_arch = "wasm32"))]`), so a
guest crate that also compiles natively for its handler tests can list it
unconditionally: the dependency resolves on both targets and contributes
nothing to the component.

## Reviewing the crate

CI checks the workspace with `--all-features`; the per-feature builds a
consumer sees are checked by hand:

```sh
for features in build guest host guest,host; do
  cargo clippy -p omnia-test --no-default-features --features "$features" --all-targets -- -D warnings
done
```

The `build` feature must stay `std`-only so a consumer's `build.rs` does not
pull the runtime into its build-dependency graph; the tree guard prints
exactly one line:

```sh
cargo tree -p omnia-test --no-default-features --features build -e normal
```
