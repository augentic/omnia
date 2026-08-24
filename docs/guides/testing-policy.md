# Seam Suite and Testing Policy

How the Omnia repository tests *itself*: the consolidated seam suite, its guest artifacts, and the coverage rules contributors follow. If you are building an application on Omnia and want to test your own guests, start with [Testing Your Guests](testing.md) instead. The binding rules are codified in the repository `AGENTS.md` (Testing policy); this page is the practical walk-through.

## The seam suite

All of the repository's seam tests live in one unpublished package, `crates/seam-suite`, compiled into a single integration-test binary (`tests/seam/main.rs` plus one module per scenario). Running them in one process lets every scenario share:

- one tokio runtime (`fixture::RT`),
- one conformance runtime — component, linker, and `InstancePre` built once (`fixture::conformance()`),
- probe handles onto every shared in-memory backend, so tests assert host-side effects.

The conformance guest (`examples/conformance/guest.rs`) exposes one HTTP route per WASI interface and imports the real guest APIs. Scenarios that need their own deployment shape (CLI, model completion/workspace, HTTP routing, MCP, typed guest API, guest-to-guest linking) build their own runtime from their own guest but still share the suite process.

Tests sharing the conformance backends take their keys/ids from `fixture::unique(..)` so concurrent scenarios never collide. The suite's shared fixture (`crates/seam-suite/tests/seam/fixture.rs`) is the exemplar for the bundle/runtime/probe pattern described in [Testing Your Guests](testing.md#anatomy-of-a-seam-test).

## Guest artifacts are explicit

Tests never invoke Cargo. `find_guest` is locate-only and fail-fast: it looks for a serialized `.bin` (preferred, loaded via deserialization instead of JIT compilation) or a `.wasm` under the example target directory and panics with build instructions when neither exists.

Build (and serialize) exactly the guests the seam suite drives with:

```bash
cargo make test-guests
```

`cargo make test-seam` depends on that task, so the one-command path is just `test-seam`. The full example set (including guests without seam coverage) still builds with `cargo make examples` for main/scheduled validation. The testkit's `guests` binary is what `test-guests` invokes to precompile built `.wasm` guests into `.bin` components via Omnia's compile path.

## Running the tiers

```bash
cargo make test        # pure tier: Nextest, excludes the seam suite
cargo make test-seam   # seam tier: builds + serializes guests, then one-process suite
cargo test --doc --all-features --workspace   # doc tests
```

`cargo-nextest` must be installed with `--locked` (`cargo install --locked cargo-nextest`). The Nextest default filter (`.config/nextest.toml`) excludes `omnia-seam-suite`, so `cargo nextest run --all` never accidentally runs seam tests process-per-test — and never silently skips a missing guest either: a seam run with missing artifacts fails with build instructions.

## Coverage rules

- **Unit tests only for pure, deterministic logic** (parsers, codecs, filter/type translation, macro token expansion). Anything crossing a WASI interface, a host backend, or dispatch is tested at the seam. Guest-side logic (`crates/omnia-guest`) keeps native unit tests since coverage tooling cannot instrument the guest `.wasm`.
- **Seam tests are the spec.** When an RFC and a seam test disagree, the seam test wins and the RFC receives an erratum in its status note.
- **Replace, then delete.** Remove a superseded unit-test module in the same change as the seam test that covers it. For non-trivial deletions, back the removal with `cargo llvm-cov` before/after evidence; a trivial deletion (an exact mirror of a seam scenario, or a strict subset of a kept test) needs only its rationale in the change.
- **Names identify, comments explain.** A test name is the scenario (`set_then_get`), not a restated expectation.

## The pure/seam boundary audit

The unit-test surface was audited against the seam tier with a `cargo llvm-cov` diff (seam tier alone vs pure tier alone). Every remaining unit-test module is a deliberate keeper, annotated with a one-line `//` comment at the module head stating why, so future audits do not relitigate. The decisions:

| Module | Decision | Rationale |
| ------ | -------- | --------- |
| `omnia/src/dispatch/handle.rs` | Migrated, unit module deleted | Depth cap and uncapped inheritance now proven at the seam (`guest_link::dispatch_depth_capped`, `dispatch_uncapped_nested_hops`) |
| `omnia/src/registry.rs` | Migrated, unit module deleted | Empty static/dynamic deployment assembly now proven at the seam (`guest_link::static_empty_deployment_rejected`, `dynamic_empty_deployment`) |
| `omnia/src/runtime/entry.rs` | Keep | `plan()` is pure argv/env parsing; the downstream run behavior is covered in `seam/cli.rs` |
| `omnia/src/telemetry.rs` | Keep | Pins the tracing/OTLP SDK contract (host-side plumbing, not the `wasi:otel` seam) |
| `omnia/src/deployment/manifest.rs` | Keep | Pure translation: TOML/JSON to `Manifest` |
| `omnia/src/registry/routing.rs` | Keep | Pure route-table matching logic |
| `wasi-model` `answer.rs` / `gate.rs` / `prompt.rs` | Keep | Pure parser / validation / string composition |
| `host-macros` | Keep | Token expansion snapshots; macros cannot cross a seam |
| `omnia-guest` (all modules) | Keep | Guest-side carve-out: `llvm-cov` cannot instrument the guest `.wasm` |

Rerun the coverage diff once per release as the drift check: any host-side line reachable only from a unit test is either a new keeper (annotate it) or a migration candidate.

## Ratchet rule

Coverage only moves toward the boundary, never away from it:

- **New host-side behavior lands with its seam scenario in the same PR.** If the change is observable through a WASI interface, the seam suite gets the test — not a unit module with mocks.
- **New backend mappings (in `omnia-backends`) land with a live case.** A translation unit test alone is not acceptance; the `#[ignore]`-gated live tier must prove the real service accepts the mapping.
- **Keepers stay annotated.** A unit-test module without a keeper comment is presumed to be a migration candidate at the next audit.
