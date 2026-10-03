# Agents

## Cursor Cloud specific instructions

### Overview

Omnia is a Rust monorepo (23 workspace crates + `examples`) providing a lightweight WASM (WASI) component runtime. Embedders depend on the `omnia` composition root, which owns deployment assembly and process lifecycle, re-exports the `omnia-core` live-runtime SDK, the `omnia-link` linking crate, the `omnia-plugin` capability crate, the `omnia-cli` leaf grammar crate (behind the `cli` feature), and the `runtime!` macro under one root; a deployment never depends on `omnia-core`, `omnia-link`, `omnia-plugin`, or `omnia-cli` directly, and code or docs that would require it are a bug. All WASI interfaces ship with in-memory defaults—no external services (Redis, NATS, Kafka, etc.) are needed for building, testing, or running examples.

Terminology (**runtime core**, **host-side**, **host-injected tools**, etc.) is defined in [docs/glossary.md](docs/glossary.md).

### Key commands

| Task         | Command                                                                                                         |
| ------------ | --------------------------------------------------------------------------------------------------------------- |
| Build        | `cargo build --all-features`                                                                                    |
| Lint         | `cargo clippy --all-features`                                                                                   |
| Format check | `cargo +nightly fmt --all --check`                                                                              |
| Format fix   | `cargo +nightly fmt --all`                                                                                      |
| Test a crate | `cargo nextest run -p <crate> --all-features` (the local verification step)                                     |
| Test (full)  | `mise run test` (`cargo nextest run --workspace --all-features`; includes the examples gate — CI's job)         |
| Lint (full)  | `mise run lint` (clippy natively, then `--target wasm32-wasip2` over lib, bins and examples; CI's job)          |
| Doc tests    | `cargo test --doc --all-features --workspace`                                                                   |
| Task runner  | `mise run <task>` (`mise tasks` lists them; `mise.toml` includes the shared Rust tasks from `augentic/toolkit`) |

### Verifying a change

- Run the suite of the crate you changed (`cargo nextest run -p <crate> --all-features`), `cargo clippy` (natively, and `--target wasm32-wasip2` for guest-side code), and `cargo +nightly fmt --all --check`. `mise run test` is the full run, examples gate included; leave it to CI.
- **Never build or run examples to check a change.** Not `cargo build --example`, not `cargo build -p examples`, not `--examples --target wasm32-wasip2`, not `cargo run --example`, not the examples gate. Examples are demos for humans: the gate takes minutes per example and asserts nothing but exit 0, so an example that builds or runs confirms neither a behaviour nor a trait bound. An agent builds or runs an example only when the user explicitly asks for that.
- Every question of the form "does this work from a real guest?" — a `wasi-*` host, `omnia-sdk`, `guest-macros`, a guest-side library, including compile-time properties such as an instrumented `async fn` satisfying axum's `Send` `Handler` bound — is answered by a `test-programs` guest:
  1. Write `crates/test-programs/programs/<capability>/<scenario>.rs`: `#![cfg(target_arch = "wasm32")]`, entered through `omnia_sdk::command!(scenario)`, asserting what the guest observes and panicking (trapping) on failure. On `wasm32` the crate already depends on what a guest needs (`axum`, `futures`, `omnia-wasi-*`, `tracing`, `wit-bindgen`, ...).
  2. Add `async fn <capability>_<scenario>()` to the suite that invokes `test_programs::foreach_<capability>!()` — `crates/wasi-<capability>/tests/<capability>.rs` for a `wasi-*` host; `command` is `crates/omnia-test/tests/command.rs`, `plugins` is `crates/omnia-plugin/tests/plugins.rs`, `link` is `crates/omnia/tests/link.rs`. It runs `test_programs::<CAPABILITY>_<SCENARIO>` through the suite's `run_guest` and asserts host-side effects; the macro fails to compile until program and test pair up.
  3. `cargo nextest run -p <host crate> --all-features`. The `test-programs` build script compiles the guest for `wasm32-wasip2`, regenerates the `[[example]]` list in `crates/test-programs/Cargo.toml` from the `programs/` tree, and emits the path constant. No separate build, no `--target` flag, no example. A guest that no longer compiles fails this build — that is the signal for a compile-time regression.

  Exemplars: `programs/otel/axum_handler.rs` (a trait bound pinned from a real guest) and `programs/model/*` with `crates/wasi-model/tests/model.rs`.

### Running examples

Examples are demos a human reads and runs by hand; they are not fixtures and not a verification step (see [Verifying a change](#verifying-a-change)). The pattern is: build the WASM guest, then run the native host runtime.

```
cargo build --example <name>-wasm --target wasm32-wasip2
cargo run --example <name> -- run ./target/wasm32-wasip2/debug/examples/<name>_wasm.wasm
```

For the HTTP example, the server listens on `localhost:8080`.

### Testing policy

The practical walk-through is [docs/guides/testing-policy.md](docs/guides/testing-policy.md). In short:

- **End-to-end tests are the primary tier for `wasi-*` host crates**: a real guest component from `crates/test-programs` (compiled by that crate's own build script) driven through omnia's own runtime (`omnia_test::host`) against an inline scenario backend, in one flat file per interface in the host crate's root `tests/` directory. Exemplar: `crates/wasi-model/tests/model.rs`, which invokes `test_programs::foreach_model!` so every guest program must have a matching test.
- **Unit tests for deterministic logic, wherever it lives**: parsers, codecs, filter/type translation, route matching, macro token expansion, guest-side library code. If a behavior is a pure function no guest boundary reaches, it is a unit test next to that logic.
- **Guest-instantiating tests exist only through the `test-programs` pipeline.** Never compile, deserialize, or instantiate a WASM guest ad hoc inside an individual test.
- **Consolidate, and drive triggers in-process.** One guest chains a whole flow (not one guest per WIT method), each outcome is asserted in exactly one place, only Omnia code is under test (never a wrapped library), and trigger hosts are exercised via `Deployment::boot` plus the trigger crate's public in-process handler rather than a server-mode boot; see the Principles section of the guide.
- **Production backends** (the `omnia-backends` repo) are accepted by `#[ignore]`-gated live tests against the real service, not by mapping unit tests alone.
- **Examples are gated by `examples/tests/examples.rs`**, which builds the guests for `wasm32-wasip2` and runs `cargo run --example` from the workspace root for the run-to-completion examples, asserting exit 0. Examples assume the default `target/` directory (wasmtime's stance; a redirected `CARGO_TARGET_DIR` is unsupported here) — hermetic guest builds belong to the `test-programs` pipeline. The gate instantiates no guest itself and asserts no behaviour (that lives in the e2e suites); server examples are build-only. It is CI's tier, never a local verification step and never a stand-in for a guest program (see [Verifying a change](#verifying-a-change)).
- **Names identify, comments explain.** A test name is the scenario (`set_then_get`), not a restated expectation (`set_then_get_round_trips`).

### Gotchas

- `cargo-nextest` must be installed with `--locked` (`cargo install --locked cargo-nextest`); without it the build fails.
- Tasks run through [mise](https://mise.jdx.dev/getting-started.html), which must be installed by hand; `make <task>` is only a pass-through to `mise run <task>` and fails, rather than installing it, when it is missing.
- Formatting uses `cargo +nightly fmt`, not stable rustfmt (the nightly toolchain must be installed).
- The `rust-toolchain.toml` pins the stable channel and auto-installs the `wasm32-wasip2` target plus `clippy`, `rust-src`, and `rustfmt` components.
- `edition = "2024"` and `rust-version = "1.95"` are workspace settings; ensure the stable toolchain is at least 1.95.
- Guest WASM examples compile to `wasm32-wasip2`; the binary name uses underscores (e.g., `http_wasm.wasm` not `http-wasm.wasm`).
- `examples` is a workspace member, so `mise run test` and `cargo nextest run --all` run the examples gate, which builds every example guest for `wasm32-wasip2` at minutes per example. Verify per crate (`-p <crate>`).
- The `[[example]]` list in `crates/test-programs/Cargo.toml` is generated by that crate's build script from `programs/`: a guest program is added by adding its file, and the list follows on the next build.

### Code comments

The shared rules under [Code style](#code-style) below apply, the workspace lints (`missing_docs` plus clippy `pedantic`/`missing_errors_doc`, all enforced via `-D warnings` in `mise run lint`) hold a doc comment on every public item and an `# Errors` section on every public fallible function, and `// SAFETY:` (linted) and `// SECURITY:` stay as they are. The `examples` crate does not inherit the workspace lints, so prefer no doc comment over one that merely echoes a handler's name.

## Git

Never `git commit`, `git push`, open or close a pull request, or delete a branch — in this repository or in any sibling checkout — unless the maintainer lifts this for the session, explicitly and for named work. Leave every change uncommitted in the working tree; the maintainer reviews and commits. No plan or to-do list carries a commit, push, or PR step, and an instruction to complete every step does not override this.

## Code style

clippy (`make lint`) and nightly rustfmt (`make fmt`) are the style gate; beyond them and the rules below, match the surrounding code.

- Suppress a lint with `#[expect(lint, reason = "…")]` at the smallest scope, never `#[allow]`.
- `<module>.rs` plus `<module>/<child>.rs`; `mod.rs` only under `tests/support/`.
- A fn over a type is that type's method, not a free fn taking it as its first argument, where the type's module declares the fn or the fn is a plain lookup or predicate on the type. A constructor is an associated fn. A policy `const` sits beside the type whose method reads it. Values several fns thread through every call become one struct whose methods they are. A fn stays free when it is pure over primitives and iterators, or when it is one module's rule applied to another module's type.

Comments follow the conventions `std`, `serde`, and `tokio` converge on: docs state the observable contract for the crate's user, never the body's mechanics.

- `///` goes on the public API only — the `pub` types, fns, fields, variants, and re-exports a user of the crate can reach — never on a private or `pub(crate)` item, an `impl` block, or a trait-impl method. A clap field's `///` is its `--help` text. A doc opens with one summary sentence (about fifteen words, full stop), then a blank line, then short sentences and bullet lists. `# Examples` holds compiled doctests, for non-obvious usage only; `# Errors` names each class the caller matches on, linked; `# Panics` the rest. Every item mentioned is an intra-doc link. No mechanics, history, or migration notes. A `//!` says what a module is for, in the same shape.
- A private item takes a `//` only for what a senior developer would not see from its name and signature: a constraint, a why, an invariant. Most carry nothing. No restatements, match-arm labels, or body paraphrases.
- Inside a body, a `//` is a section header: lowercase, no full stop, above a blank-line-separated block, naming what the block achieves, so the headers read together outline the fn. A fn readable at a glance carries none, and a header never narrates the line beneath it. The one in-body explanation is `// HACK: …`, for a trick a senior would not see through.
- A test fn takes `//`, never `///`, and only for rationale its scenario name and assertions do not expose.
- No commented-out code.
- Every sentence earns its place and reads once: short plain sentences, one idea each; three or more things are a bullet list, not a colon-and-dash clause; no chained em-dashes, nested parentheticals, or semicolon runs; each fact has one home across `//!`, `///`, and `//`. A comment is as long as its why takes and no longer — concise is not dense, and readable is not verbose.

## Testing

Tests drive the public boundary: a behaviour is asserted through what a user of the product or crate can reach, over scripted doubles rather than a live filesystem, network, or model, never through private internals. A suite below the root survives only for an independent library contract; a unit test only for a branch no public boundary reaches. A test fn names the scenario (`gen_spec`, `no_sources`), never the outcome. Scripted doubles are strict: script exactly the exchanges a run consumes.

## Commands

All from the repository root through `make` ([`Makefile`](Makefile) → mise). The tasks are the shared `mise/rust.toml` of [`augentic/toolkit`](https://github.com/augentic/toolkit), pinned in [`mise.toml`](mise.toml) to the tag every `uses:` under `.github/workflows/` names; bump both together in one pull request.

```bash
make ci # exactly the CI jobs: fmt-check + lint + test + test-docs + docs + vet + deny  — run before handing over
make check # local advisories: audit + fmt (rewrites) + lint + outdated + deps
make test # cargo nextest run --locked --workspace --all-features, under -Dwarnings
make lint # lint-host (cargo clippy --workspace --all-targets --all-features, then cargo hack --each-feature), then lint-wasm (the same over every lib, bin and example for wasm32-wasip2 — never tests)
make fmt # cargo +nightly fmt --all
make vet-regen # regenerate cargo-vet imports/exemptions/unpublished, then vet
make cov # cargo llvm-cov nextest --workspace --all-features --summary-only
make sweep # drop target/ artifacts untouched for a week
```

If `make ci` cannot run, say exactly why and which checks ran instead.

