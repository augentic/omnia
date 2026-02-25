# Agents

## Cursor Cloud specific instructions

### Overview

Omnia is a Rust monorepo (17 workspace crates + `examples`) providing a lightweight WASM (WASI) component runtime. All WASI interfaces ship with in-memory defaults—no external services (Redis, NATS, Kafka, etc.) are needed for building, testing, or running examples.

### Key commands

| Task | Command |
|------|---------|
| Build | `cargo build --all-features` |
| Lint | `cargo clippy --all-features` |
| Format check | `cargo +nightly fmt --all --check` |
| Format fix | `cargo +nightly fmt --all` |
| Test | `cargo nextest run --all --all-features --no-tests=pass` |
| Doc tests | `cargo test --doc --all-features --workspace` |
| Task runner | `cargo make <task>` (see `Makefile.toml` for available tasks) |

### Running examples

Examples follow a two-step pattern: build the WASM guest, then run the native host runtime.

```
cargo build --example <name>-wasm --target wasm32-wasip2
cargo run --example <name> -- run ./target/wasm32-wasip2/debug/examples/<name>_wasm.wasm
```

For the HTTP example, the server listens on `localhost:8080`.

### Gotchas

- `cargo-nextest` must be installed with `--locked` (`cargo install --locked cargo-nextest`); without it the build fails.
- Formatting uses `cargo +nightly fmt`, not stable rustfmt (the nightly toolchain must be installed).
- The `rust-toolchain.toml` pins the stable channel and auto-installs the `wasm32-wasip2` target plus `clippy`, `rust-src`, and `rustfmt` components.
- `edition = "2024"` and `rust-version = "1.93"` are workspace settings; ensure the stable toolchain is at least 1.93.
- Guest WASM examples compile to `wasm32-wasip2`; the binary name uses underscores (e.g., `http_wasm.wasm` not `http-wasm.wasm`).
