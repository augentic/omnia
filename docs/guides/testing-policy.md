# Testing Policy

How the Omnia repository tests *itself*. If you are building an application on Omnia and want to test your own guests, start with [Testing Your Guests](testing.md) instead. The binding rules are codified in the repository `AGENTS.md` (Testing policy); this page is the practical walk-through.

## The tiers

- **Unit tests** cover deterministic logic wherever it lives: parsers, codecs, filter/type translation, route matching, macro token expansion, and backend semantics driven directly against a `WasiXxxCtx` trait. If a behavior can be pinned without instantiating a guest, it is a unit test.
- **ABI tests** cover behavior that *is* the guest–host boundary. They live in [`crates/abi-tests`](../../crates/abi-tests) as ordinary integration tests — one auto-discovered target per scenario family — and run under Nextest process-per-test alongside everything else. No shared fixtures, no cross-test key discipline: each test builds its own runtime from the serialized guest artifacts, which is cheap.
- **Live tests** (in the `omnia-backends` repo) are the acceptance tier for production backends: `#[ignore]`-gated, credential-gated, driving the backend's `WasiXxxCtx` against the real service.

## What earns an ABI test

An ABI test earns its keep only when the contract cannot be observed anywhere cheaper:

- the host mediates between guests — dispatch depth, timeouts, registration lifecycle (`tests/guest_link.rs`);
- the host owns a session against a backend — model tool sessions, budgets, cancellation, workspace identity matching (`tests/model.rs`);
- a policy is applied on the way out of the sandbox — outbound header stripping, client certificates (`tests/conformance.rs`);
- a resource or typed error threads across WIT — the keyvalue `cas-failed` fresh handle (`tests/conformance.rs`);
- a trigger delivers inbound events to a guest export — the websocket handler leg (`tests/conformance.rs`);
- artifact acquisition and trust — bytes-sourced guests, the pre-compiled policy (`tests/embedded.rs`);
- CLI command routing and exit mapping (`tests/cli.rs`).

One test per contract. Assert the guest-visible outcome; add a host-side **probe** only when the guest cannot observe the effect itself — a message that must land on the broker, a frame that must reach a connected peer, a write that must be denied host-side. A probe that re-reads the same in-memory store the guest just wrote and read back adds no information; don't write it.

"The linker still wires this import" is not a contract worth a dedicated test: every conformance scenario already fails loudly if linking breaks, and the in-memory defaults are trivial. When a new WASI interface lands, it needs an ABI test only if it carries one of the behaviors above.

## Guest artifacts are explicit

Tests never invoke Cargo. `find_guest` is locate-only and fail-fast: it looks for a serialized `.bin` (preferred, loaded via deserialization instead of JIT compilation) or a `.wasm` under the example target directory and panics with build instructions when neither exists.

Build (and serialize) exactly the guests the suite drives with:

```bash
cargo make test-guests
```

`cargo make test` depends on that task, so the one-command path is just `test`. The full example set still builds with `cargo make examples` for main/scheduled validation. The `omnia-abi-tests` `guests` binary is what `test-guests` invokes to precompile built `.wasm` guests into `.bin` components via Omnia's compile path.

## Running

```bash
cargo make test        # builds + serializes the ABI-test guests, then `cargo nextest run --all`
cargo test --doc --all-features --workspace   # doc tests
```

`cargo-nextest` must be installed with `--locked` (`cargo install --locked cargo-nextest`). A bare `cargo nextest run --all` without pre-built guests fails fast with build instructions — never a silent skip.
