# Agents

## Cursor Cloud specific instructions

### Overview

Omnia is a Rust monorepo (17 workspace crates + `examples`) providing a lightweight WASM (WASI) component runtime. All WASI interfaces ship with in-memory defaults—no external services (Redis, NATS, Kafka, etc.) are needed for building, testing, or running examples.

Terminology (**runtime core**, **host-side**, **host-injected tools**, etc.) is defined in [docs/glossary.md](docs/glossary.md).

### Key commands

| Task             | Command                                                                  |
| ---------------- | ------------------------------------------------------------------------ |
| Build            | `cargo build --all-features`                                             |
| Lint             | `cargo clippy --all-features`                                            |
| Format check     | `cargo +nightly fmt --all --check`                                       |
| Format fix       | `cargo +nightly fmt --all`                                               |
| Test (pure tier) | `cargo nextest run --all --all-features --no-tests=pass`                 |
| Seam guests      | `cargo make test-guests` (build + serialize the seam-suite guests)       |
| Test (seam tier) | `cargo make test-seam` (or `cargo test -p omnia-seam-suite --test seam`) |
| Doc tests        | `cargo test --doc --all-features --workspace`                            |
| Task runner      | `cargo make <task>` (see `Makefile.toml` for available tasks)            |

### Running examples

Examples follow a two-step pattern: build the WASM guest, then run the native host runtime.

```
cargo build --example <name>-wasm --target wasm32-wasip2
cargo run --example <name> -- run ./target/wasm32-wasip2/debug/examples/<name>_wasm.wasm
```

For the HTTP example, the server listens on `localhost:8080`.

### Testing policy (integration-first)

The practical walk-through is [docs/guides/testing.md](docs/guides/testing.md). In short:

- **Unit tests only for pure, deterministic logic** (parsers, codecs, filter/type translation, macro token expansion). Anything crossing a WASI interface, a host backend, or dispatch is tested at the guest–host seam.
- **Seam tests are the spec.** All seam tests live in the consolidated single-process suite [crates/seam-suite](crates/seam-suite) (one `tests/seam` binary, one module per scenario). Most scenarios drive the shared conformance guest ([examples/conformance/guest.rs](examples/conformance/guest.rs)) through the shared runtime fixture and assert host-side effects via probe handles (see `tests/seam/fixture.rs`); scenarios needing their own deployment shape (CLI, model, routing, MCP, guest linking) build their own runtime in their module. Drive HTTP guests with `omnia_testkit::http`; use `omnia_testkit::temp_manifest` for manifest-driven setups.
- **Guest artifacts are explicit.** Tests never invoke Cargo. Run `cargo make test-guests` first (builds and serializes the seam guests); `find_guest` locates artifacts and fails fast with build instructions when one is missing — no silent skips. The Nextest default filter (`.config/nextest.toml`) excludes `omnia-seam-suite` from the pure tier.
- **Replace, then delete.** Remove a superseded unit-test module in the same change as the seam test that covers it, with `cargo llvm-cov` before/after evidence that coverage holds. Guest-side logic (`crates/omnia-guest`) keeps native unit tests since `llvm-cov` can't instrument the guest `.wasm`.
- **Names identify, comments explain.** A test name is the scenario (`set_then_get`), not a restated expectation (`set_then_get_round_trips`).

### Gotchas

- `cargo-nextest` must be installed with `--locked` (`cargo install --locked cargo-nextest`); without it the build fails.
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
