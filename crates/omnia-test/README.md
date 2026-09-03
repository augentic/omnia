# omnia-test

Test doubles, a component runtime harness, and a `wasm32-wasip2` fixture
pipeline for code built on omnia — one native-only crate with three additive
features.

| Feature | Carries | Depends on |
| ------- | ------- | ---------- |
| `guest` | Native doubles for every `omnia_guest` capability trait, the `provider!` and `delegate!` macros | `omnia-guest` (with `orm`) |
| `host` | `Deployment`, `Backends`, `ScriptedModel`, `Scratch` — the component runtime harness | `omnia` and the `wasi-*` host crates |
| `build` | `Components` — the nested wasm32 build and `gen.rs` generator for a `build.rs` | `std` only |

There is no default feature: a consumer names the rung it uses. All three
share one `Script` core, so a scripted model reads the same at the handler
rung (`guest::Scripted`) and the component rung (`host::ScriptedModel`). The
walk-through is [Testing Omnia-Based Code](https://github.com/augentic/omnia/blob/main/docs/guides/testing-omnia-code.md).

`omnia_test::provider!` deliberately shares its name and grammar with
`omnia_guest::provider!`: the production declaration expands to WASI-backed
impls for `wasm32`, the test declaration to native doubles, and the two differ
by the crate path alone.

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
all. The crate is also empty on `wasm32` (`#![cfg(not(target_arch =
"wasm32"))]`), so an ungated line resolves on both targets and contributes
nothing to the component — the gate simply keeps the host crates out of the
`wasm32` dependency graph.

## Reviewing the crate

`cargo make ci` checks the shapes a consumer sees, not just `--all-features`:

```sh
cargo make features    # clippy per feature: build, guest, host, guest,host
cargo make hack        # cargo hack check --feature-powerset --no-dev-deps
cargo make tree-guard  # `build` resolves to std alone
cargo make semver      # cargo semver-checks against the last release
```

The `build` feature must stay `std`-only so a consumer's `build.rs` does not
pull the runtime into its build-dependency graph; `tree-guard` fails unless
`cargo tree -p omnia-test --no-default-features --features build -e normal`
prints exactly one line.
