# Agents

## Cursor Cloud specific instructions

### Overview

Omnia is a Rust monorepo (23 workspace crates + `examples`) providing a lightweight WASM (WASI) component runtime. Embedders depend on the `omnia` composition root, which owns deployment assembly and process lifecycle, re-exports the `omnia-core` live-runtime SDK, the `omnia-link` linking crate, the `omnia-plugin` capability crate, the `omnia-otlp` exporter crate (behind the `otlp` feature), the `omnia-cli` leaf grammar crate (behind the `cli` feature), and the `runtime!` macro under one root; a deployment never depends on `omnia-core`, `omnia-link`, `omnia-plugin`, `omnia-otlp`, or `omnia-cli` directly, and code or docs that would require it are a bug. All WASI interfaces ship with in-memory defaults—no external services (Redis, NATS, Kafka, etc.) are needed for building, testing, or running examples.

Terminology (**runtime core**, **host-side**, **host-injected tools**, etc.) is defined in [docs/glossary.md](docs/glossary.md).

### Key commands

| Task         | Command                                                                                              |
| ------------ | ---------------------------------------------------------------------------------------------------- |
| Build        | `cargo build --all-features`                                                                         |
| Lint         | `cargo clippy --all-features`                                                                        |
| Format check | `cargo +nightly fmt --all --check`                                                                   |
| Format fix   | `cargo +nightly fmt --all`                                                                           |
| Test a crate | `cargo nextest run -p <crate> --all-features` (the local verification step)                          |
| Test (full)  | `cargo make test` (`cargo nextest run --all --all-features`; includes the examples gate — CI's job)  |
| Doc tests    | `cargo test --doc --all-features --workspace`                                                        |
| Task runner  | `cargo make <task>` (see `Makefile.toml` for available tasks)                                        |

### Verifying a change

- Run the suite of the crate you changed (`cargo nextest run -p <crate> --all-features`), `cargo clippy` (natively, and `--target wasm32-wasip2` for guest-side code), and `cargo +nightly fmt --all --check`. `cargo make test` is the full run, examples gate included; leave it to CI.
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
- Formatting uses `cargo +nightly fmt`, not stable rustfmt (the nightly toolchain must be installed).
- The `rust-toolchain.toml` pins the stable channel and auto-installs the `wasm32-wasip2` target plus `clippy`, `rust-src`, and `rustfmt` components.
- `edition = "2024"` and `rust-version = "1.95"` are workspace settings; ensure the stable toolchain is at least 1.95.
- Guest WASM examples compile to `wasm32-wasip2`; the binary name uses underscores (e.g., `http_wasm.wasm` not `http-wasm.wasm`).
- `examples` is a workspace member, so `cargo make test` and `cargo nextest run --all` run the examples gate, which builds every example guest for `wasm32-wasip2` at minutes per example. Verify per crate (`-p <crate>`).
- The `[[example]]` list in `crates/test-programs/Cargo.toml` is generated by that crate's build script from `programs/`: a guest program is added by adding its file, and the list follows on the next build.

### Code comments

Golden rule: do not document what is self-evident in code. Note, however, that the workspace lints (`missing_docs` plus clippy `pedantic`/`missing_errors_doc`, all enforced via `-D warnings` in `cargo make lint`) require a doc comment on every public item and an `# Errors` section on every public fallible function. Within that constraint:

- Keep public-item docs to a concise one-line summary; do not pad them by restating the signature, types, or fragile cross-references that a glance at the code already shows.
- Do not attach doc-comment labels to `impl` blocks (for example `From` conversions) — impl blocks need no docs, so a `/// X to Y mapping` line is pure noise.
- Inline comments (`//`) are never linted: add them only to explain *why* (security, performance, non-local control flow), never to narrate *what* the next line does.
- Trim redundant secondary sentences from multi-line docs, keeping the summary line the lint requires.
- The `examples` crate does not inherit the workspace lints, so prefer no doc comment over one that merely echoes a handler's name.
