# Testing Policy

How the Omnia repository tests *itself*. The binding rules are also in the repository `AGENTS.md` (Testing policy); this page is the practical walk-through. For testing code *built on* omnia — a guest crate or an embedder — see [Testing Omnia-Based Code](testing-omnia-code.md), which covers the published `omnia-test` crate the suites below are built on.

## The tiers

- **End-to-end tests** are the primary tier for `wasi-*` host crates: a real guest component driven through omnia's own runtime against an inline scenario backend, pinning the whole boundary (guest bindings → linker → host binding → backend and back). The exemplar is `crates/wasi-model/tests/model.rs`.
- **Unit tests** cover deterministic logic wherever it lives: parsers, codecs, filter/type translation, route matching, macro token expansion, guest-side library code. If a behavior is a pure function no guest boundary reaches (e.g. `Format::candidate` extraction, which backends drive directly), it is a unit test next to that logic.
- **Live tests** (in the `omnia-backends` repo) are the acceptance tier for production backends: `#[ignore]`-gated, credential-gated, driving the backend's `WasiXxxCtx` against the real service.
- **The examples gate** (`examples/tests/examples.rs`) builds the guests for `wasm32-wasip2` and runs `cargo run --example` from the workspace root for the run-to-completion examples, asserting exit status 0. Examples assume the default `target/` directory (wasmtime's stance; a redirected `CARGO_TARGET_DIR` is unsupported for this tier) — hermetic guest builds belong to the `test-programs` pipeline. The gate instantiates no guest of its own and asserts no behaviour — that lives in the e2e suites — so server examples are build-only. It is CI's tier: minutes per example, and never a local verification step.

Guest-instantiating tests exist **only** through the shared pipeline below. Do not compile, deserialize, or instantiate a WASM guest ad hoc inside an individual test, and do not build or run an example to check that something works from a guest — an example is a demo, and a passing one confirms neither a behaviour nor a trait bound. That question is always answered by [adding a guest program](#adding-a-guest-program).

## Principles

- **Test Omnia code only.** A wrapped library (wasmtime-wasi-http, SQLite, tungstenite, the OpenTelemetry SDK) is never the subject of a guest or a unit test. If an assertion would still hold with Omnia's binding replaced by a pass-through, it is testing the library, not us.
- **One `test-programs` crate.** Every guest lives under `crates/test-programs/programs/<capability>/`; there is no second fixture crate and no per-host artifact split. A capability's guests and its host suite are the only two places its e2e coverage exists.
- **Chain, then stop.** One guest walks a flow end to end with several asserts along the way (open → write → read → list → delete). A store host gets one or two guests, not one per WIT method; a guest is minted for a *scenario*, not a function.
- **One owner per outcome.** Each observable outcome — a value crossing the boundary, a persisted side effect, a recorded backend call — is asserted in exactly one place. When a guest uniquely observes an outcome, the unit test that used to cover it goes. What remains as unit tests is leftover pure logic no boundary reaches: parsers, codecs, filter evaluation, header rules.
- **Triggers are driven in-process.** The [trigger](../glossary.md#trigger) hosts (HTTP incoming, messaging incoming-handler, websocket handler) are tested by `Deployment::boot` plus the trigger crate's public in-process handler (`HttpHandler`, `MessagingHandler`, `WebSocketHandler`), which does the route → instantiate → invoke-export step the server loop does; no server-mode boot, no sockets. Outgoing HTTP is a command guest against a test-owned loopback mock. Model stays fine-grained (one guest per protocol behaviour) by decision, because each of its scenarios pins a distinct host-side rule.
- **Unimplemented WIT is out of scope.** An interface Omnia does not implement (for example the keyvalue watcher) has no guest and is not a coverage hole.

## The e2e pipeline

One unpublished crate, patterned on wasmtime's `test-programs`, over the published `omnia-test`. **`crates/test-programs`** is both sides of the boundary:

- On `wasm32` it is the guest scenario programs, one `[[example]]` cdylib per scenario (`programs/<capability>/<scenario>.rs`), plus their shared helpers in `src/helpers.rs`. The example, path constant, and host test identity is `<capability>_<scenario>` (`model_echo_text`). Each program asserts what the guest observes across the boundary and traps on failure, and enters through `omnia_sdk::command!(scenario)`.
- Natively it is the compiled artifacts. Its `build.rs` compiles every program to a `wasm32-wasip2` component through `omnia_test::build::Components` (into `target/wasm32-fixtures`, one directory shared by every outer configuration, so plain `cargo make test` is self-contained), regenerates the `[[example]]` stanzas from the `programs/` tree, and writes `gen.rs`: one `pub const <NAME>: &str` artifact path per program plus a `foreach_<capability>!` macro, which `src/lib.rs` includes. The native side has no dependencies. The nested build compiles this same package for `wasm32` and so runs the script again; `Components` is a no-op under a `wasm32` target, which is what lets the guest package own its own fixture build.

The harness a suite drives the artifacts through is `omnia_test::host`, the same one a downstream consumer uses: `Deployment::new().guest(id, wasm).mounts(..).run_host::<H, B>(backends)` builds a one-shot `wasi:cli` command deployment and links the host under test beside `WasiOtel` (which `command!` imports; `run(backends, link)` takes a `link` closure for suites that link by hand), and `scratch()` mints a per-test workspace directory removed on drop.

A host crate's suite is one flat file per interface in its root `tests/` directory (`crates/wasi-model/tests/model.rs`). The file:

- invokes `test_programs::foreach_<capability>!();` so a guest program without a matching, identically named test fails to compile — this completeness check is the orphan guard: a guest under `programs/<capability>/` cannot exist without a host test exercising it, and a `programs/<capability>/` directory cannot exist without a suite invoking its macro;
- defines its scenario backends inline next to the tests (see below);
- runs each guest with `Deployment::run_host` (via a small local `run_guest` wrapper supplying the `Has<Capability>` bundle and requiring `ExitStatus::SUCCESS`), then asserts any host-side effects (recorded requests, persisted state read back through the `Backends` handles, filesystem contents). Suites that need more than one host, or a hand-written scenario backend in a custom bundle, link by hand with `Deployment::run`; trigger suites use `Deployment::boot` and the crate's in-process handler instead of a `wasi:cli/run` entry point.

Assertions split by vantage point: the guest asserts what crosses the boundary to it (a panic traps and fails the host test); the host test asserts wire fidelity and side effects. A side effect is asserted from one vantage point only, per "one owner per outcome" above.

One group tests the guest SDK's own boundary rather than a `wasi-*` host: `programs/command/` holds `command!` guests built on the command façade (`command/exit_map`), and `crates/omnia-test/tests/command.rs` drives them with `Deployment::run` over the default bundle, asserting the `ExitStatus` the host observes for each verb — the exit map (`ok` 0, `bad` 1, `missing` 2, `upstream` 4) and `USAGE_EXIT` (64) for an unknown verb. These programs do not trap; the exit status *is* the behaviour under test, so the guest returns a `Response` and the host test reads `code_u8()`.

## Adding a guest program

The pipeline is also how a change is *verified* when the question is "does this work from a real guest?" — whether the change is to a `wasi-*` host, to `omnia-sdk`, to `guest-macros`, or to a guest-side library, and whether the property is a runtime behaviour or a compile-time one (`programs/otel/axum_handler.rs` pins an instrumented `async fn` against axum's `Send` `Handler` bound; if the future ever stops being `Send`, the guest stops compiling and the suite fails to build).

1. **Write the program**: `crates/test-programs/programs/<capability>/<scenario>.rs`, starting with `#![cfg(target_arch = "wasm32")]` and entering through `omnia_sdk::command!(scenario)`. It asserts what the guest observes across the boundary and panics (traps) on failure. On `wasm32` the crate already depends on what a guest needs (`axum`, `futures`, `omnia-wasi-*`, `tracing`, `wit-bindgen`, ...); add a workspace dependency only if a scenario genuinely needs one. The `[[example]]` stanza is generated: the build script regenerates the list from the `programs/` tree, so the file is the only thing to add.
2. **Write the test**: `async fn <capability>_<scenario>()` in the suite that invokes `test_programs::foreach_<capability>!()` (`crates/wasi-<capability>/tests/<capability>.rs` for a `wasi-*` host; `command`, `plugins`, and `link` live in `crates/omnia-test`, `crates/omnia-plugin`, and `crates/omnia` respectively). It runs `test_programs::<CAPABILITY>_<SCENARIO>` through the suite's `run_guest` and asserts the host-side effects. Until the program and the test pair up, the macro fails to compile.
3. **Run the suite**: `cargo nextest run -p <host crate> --all-features`. The build script compiles the guest for `wasm32-wasip2` as part of that command — no separate build, no `--target` flag, no example.

## Scenario backends

The bundle a suite runs over is `omnia_test::host::Backends`: the in-memory default for every host, deterministic (no environment read, no socket opened), with the model swappable for any `WasiModelCtx`. Most model scenarios script `ScriptedModel` — the answers, the tool calls and workspace steps each completion makes before answering, and the limits — and assert the recorded exchanges afterwards:

```rust,noplayground
let model = ScriptedModel::answering(["42"]).calling(0, [("lookup", "{}")]);
run_guest(test_programs::MODEL_TOOL_ROUNDTRIP, vec![], model.clone()).await;
model.assert_exhausted();
assert_eq!(model.exchanges(), [Exchange { tool: "lookup".into(), arguments: "{}".into(), outcome: Ok("42".into()) }]);
```

A behaviour a FIFO script cannot express — two tool calls in flight at once, a backend that ignores a hard failure — is a hand-written `WasiModelCtx` defined inline next to the test, with a comment saying why the script could not do it. The in-tree echo `ModelDefault` covers scenarios where the answer does not matter, or where its schema rejection is itself under test.

The same inline pattern serves hosts whose default backend records nothing observable: `wasi-identity`, `wasi-websocket`, and `wasi-otel` each define a recording `WasiXxxCtx` next to their tests, wrapped in a local bundle providing that host plus `WasiOtel`, and assert the recorded calls after the run.

## Running

```bash
cargo nextest run -p <crate> --all-features       # the crate you changed: unit tests plus its e2e suite
cargo make test                                   # everything (`cargo nextest run --locked --all --all-features`), examples gate included
cargo test --doc --all-features --workspace       # doc tests
```

Verify locally per crate; the full run is CI's, because `examples` is a workspace member and its gate builds every example guest.

`cargo-nextest` must be installed with `--locked` (`cargo install --locked cargo-nextest`). The `wasm32-wasip2` target must be installed (`rust-toolchain.toml` pins it); `test-programs`'s build script needs it to compile the guest programs.

## Naming

A test name is the scenario (`set_then_get`), not a restated expectation (`set_then_get_round_trips`).
