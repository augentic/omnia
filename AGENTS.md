# Agents

## Cursor Cloud specific instructions

### Overview

Omnia is a Rust monorepo (17 workspace crates + `examples`) providing a lightweight WASM (WASI) component runtime. All WASI interfaces ship with in-memory defaults—no external services (Redis, NATS, Kafka, etc.) are needed for building, testing, or running examples.

Terminology (**runtime core**, **host-side**, **host-injected tools**, etc.) is defined in [docs/glossary.md](docs/glossary.md).

### Key commands

| Task         | Command                                                                                                 |
| ------------ | ------------------------------------------------------------------------------------------------------- |
| Build        | `cargo build --all-features`                                                                            |
| Lint         | `cargo clippy --all-features`                                                                           |
| Format check | `cargo +nightly fmt --all --check`                                                                      |
| Format fix   | `cargo +nightly fmt --all`                                                                              |
| Test         | `cargo make test` (builds + serializes the ABI-test guests, then `cargo nextest run --all --all-features`) |
| Test guests  | `cargo make test-guests` (build + serialize the ABI-test guests only)                                   |
| Doc tests    | `cargo test --doc --all-features --workspace`                                                           |
| Task runner  | `cargo make <task>` (see `Makefile.toml` for available tasks)                                           |

### Running examples

Examples follow a two-step pattern: build the WASM guest, then run the native host runtime.

```
cargo build --example <name>-wasm --target wasm32-wasip2
cargo run --example <name> -- run ./target/wasm32-wasip2/debug/examples/<name>_wasm.wasm
```

For the HTTP example, the server listens on `localhost:8080`.

### Testing policy

The practical walk-through is [docs/guides/testing-policy.md](docs/guides/testing-policy.md); [docs/guides/testing.md](docs/guides/testing.md) is the guide for application authors. In short:

- **Unit tests for deterministic logic, wherever it lives**: parsers, codecs, filter/type translation, route matching, macro token expansion, guest-side library code, and backend semantics driven directly against a `WasiXxxCtx` trait. If a behavior can be pinned without instantiating a guest, it is a unit test.
- **ABI tests only for behavior that is the guest–host boundary itself**: host-mediated dispatch, model tool sessions, trigger delivery, artifact acquisition/trust, CLI exit mapping, outbound HTTP policy, resources or typed errors threading across WIT. They live in [crates/abi-tests](crates/abi-tests) as ordinary integration tests (one auto-discovered target per scenario family) and run under Nextest process-per-test with the rest of the workspace — each test builds its own runtime from serialized guests. One test per contract: assert the guest-visible outcome, and add a host-side probe only when the guest cannot observe the effect (broker delivery, peer sockets, denied writes). A probe that re-reads the store the guest just round-tripped adds nothing; don't write it. Drive HTTP guests with `omnia_abi_tests::http`; use `omnia_abi_tests::temp_manifest` for manifest-driven setups.
- **Guest artifacts are explicit.** Tests never invoke Cargo. `cargo make test` builds and serializes the ABI-test guests (`cargo make test-guests`) before running Nextest; `find_guest` locates artifacts and fails fast with build instructions when one is missing — no silent skips.
- **Production backends** (the `omnia-backends` repo) are accepted by `#[ignore]`-gated live tests against the real service, not by mapping unit tests alone.
- **Names identify, comments explain.** A test name is the scenario (`set_then_get`), not a restated expectation (`set_then_get_round_trips`).

### Gotchas

- `cargo-nextest` must be installed with `--locked` (`cargo install --locked cargo-nextest`); without it the build fails.
- ABI tests need pre-built guest artifacts: a bare `cargo nextest run --all` without `cargo make test-guests` fails fast with build instructions. `cargo make test` is the one-command path.
- Formatting uses `cargo +nightly fmt`, not stable rustfmt (the nightly toolchain must be installed).
- The `rust-toolchain.toml` pins the stable channel and auto-installs the `wasm32-wasip2` target plus `clippy`, `rust-src`, and `rustfmt` components.
- `edition = "2024"` and `rust-version = "1.95"` are workspace settings; ensure the stable toolchain is at least 1.95.
- Guest WASM examples compile to `wasm32-wasip2`; the binary name uses underscores (e.g., `http_wasm.wasm` not `http-wasm.wasm`).

### Code comments

Golden rule: do not document what is self-evident in code. Note, however, that the workspace lints (`missing_docs` plus clippy `pedantic`/`missing_errors_doc`, all enforced via `-D warnings` in `cargo make lint`) require a doc comment on every public item and an `# Errors` section on every public fallible function. Within that constraint:

- Keep public-item docs to a concise one-line summary; do not pad them by restating the signature, types, or fragile cross-references that a glance at the code already shows.
- Do not attach doc-comment labels to `impl` blocks (for example `From` conversions) — impl blocks need no docs, so a `/// X to Y mapping` line is pure noise.
- Inline comments (`//`) are never linted: add them only to explain *why* (security, performance, non-local control flow), never to narrate *what* the next line does.
- Trim redundant secondary sentences from multi-line docs, keeping the summary line the lint requires.
- The `examples` crate does not inherit the workspace lints, so prefer no doc comment over one that merely echoes a handler's name.
