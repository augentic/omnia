# Design: Integration Testing in Omnia

> Status: Design proposal — no behaviour change; improves test coverage and CI
> reliability for WASM guest/host flows and proc-macro crates.

## 1. The problem

Omnia is a WASM (WASI) component runtime. Most behaviour spans a guest `.wasm`
component, the host runtime, and one or more WASI host backends. Unit tests on
individual crates cannot exercise that seam.

Today, validation falls into two buckets:

1. **Automated integration tests** — only two exist:
   - `crates/omnia/tests/linking.rs` — host-mediated dynamic linking
     (`examples/linking`)
   - `crates/wasi-model/tests/replay.rs` — record/replay and `resolve` dispatch
     (`examples/model`)
2. **Manual example runs** — everything else (`otel`, `http`, `keyvalue`, `sql`,
   `blobstore`, …) is exercised by building a guest, starting the example host,
   and inspecting logs or curling an endpoint.

The `wasi-otel-attr` macro is a concrete example of the gap. The macro lives in
`crates/wasi-otel-attr` with **zero** automated tests. To verify that
`#[omnia_wasi_otel::instrument]` expands correctly and that instrumented spans
reach the host, a developer must manually run `examples/otel`:

```bash
cargo build --example otel-wasm --target wasm32-wasip2
cargo run --example otel -- run ./target/wasm32-wasip2/debug/examples/otel_wasm.wasm
curl -d '{"text":"hello"}' http://localhost:8080
```

That pattern does not scale across 15+ WASI interfaces and proc-macro crates.

## 2. Current state

### 2.1 The existing integration-test pattern

Both `linking.rs` and `replay.rs` follow the same shape:

1. Locate a pre-built guest under
   `target/wasm32-wasip2/{debug,release}/examples/<name>_wasm.wasm`.
2. Write a temporary manifest with absolute paths to the guest(s).
3. Hand-roll a `TestCtx` / `TestRuntime` implementing the required host views.
4. Call `create_from_manifest`, link host interfaces, and drive guest exports
   via `call_async`.

The model test goes further: it swaps in a **recording/stub backend** to assert
host-side effects (fixture written, replayed answer matches, `resolve` dispatches
to a fresh `shelf` instance per call).

When the guest `.wasm` is absent, both tests **skip** (print instructions and
return `Ok(())`) rather than fail. That is intentional — `cargo make test` and
`cargo make ci` run `clean` before `nextest`, which removes any previously
built guests.

### 2.2 CI gap

`Makefile.toml` wires `test` and `ci` through `clean`:

```toml
[tasks.test]
dependencies = ["clean"]
args = ["nextest", "run", "--all", "--all-features", "--no-tests=pass"]
```

There is **no** task that builds example guests for `wasm32-wasip2` before
tests run. The only references to that target outside config are the skip
messages in the two integration tests. Unless CI (`.github/workflows/ci.yaml`,
which delegates to `augentic/.github/...@main`) builds guests after clean, the
existing integration tests pass vacuously by skipping.

### 2.3 Proc-macro testing gap

`crates/wasi-otel-attr` is a proc-macro crate with no `[dev-dependencies]` and
no test infrastructure (`trybuild`, `macrotest`, `insta`) anywhere in the repo.
The same applies to `runtime-macro` and `guest-macro`.

## 3. Proposed improvements

### 3.1 Fix the CI/build pipeline (highest leverage)

Add a `build-guests` task to `Makefile.toml` that builds all `*-wasm` examples
for `wasm32-wasip2`:

```bash
cargo build -p examples \
  --examples \
  --target wasm32-wasip2
```

Make `test` depend on `build-guests` **after** `clean` (or drop `clean` from
the default test path and reserve it for a separate `test-clean` task). Verify
the shared CI workflow runs the guest build before `nextest` so `linking` and
`replay` actually execute rather than skip.

**Outcome:** existing integration tests become meaningful in CI with no new test
code.

### 3.2 Extract shared test support

`linking.rs` and `replay.rs` duplicate roughly 100 lines each:

- `target_dir()` — derive `target/` from the test executable path
- `guest_wasm()` — locate a built guest by file name
- Manifest writing to a temp file
- A generic store-counting `TestRuntime` / `TestCtx`

Factor these into a `crates/test-support` crate (or a shared module behind a
dev-dependency) so each WASI crate's integration test is ~20 lines of
interface-specific wiring. Lowering the per-interface cost is the single biggest
lever for adding more integration tests.

### 3.3 Proc-macro tests for `wasi-otel-attr`

The macro has two distinct concerns, best tested at two tiers.

#### Tier 1 — expansion and arg parsing (fast, native, no WASM)

The interesting logic lives in `body()` and `Attributes` in
`crates/wasi-otel-attr/src/lib.rs`, which operate on `proc_macro2` / `syn`
tokens (not `proc_macro::TokenStream`). Call `body()` directly from a
`#[cfg(test)] mod tests` in the crate:

| Input | Expected expansion |
|-------|-------------------|
| Async fn, no args | `Instrument::instrument(async move …, span!(INFO, fn_name)).await` |
| Sync fn, no args | `span!(INFO, fn_name).in_scope(\|\| { … })` |
| `name = "custom"` | Span name is `"custom"` |
| `level = Level::DEBUG` | Level propagates into `span!` |

For the full macro surface (including the `proc_macro` boundary and compile-time
errors), add `trybuild` as a dev-dependency with a `tests/ui/` folder:

- **Pass cases:** named span, leveled span, async/sync functions
- **Fail case:** `#[instrument(bogus = 1)]` → `"unsupported property"`

This generalizes to `runtime-macro` and `guest-macro`.

#### Tier 2 — behavioural (does instrumentation export spans?)

Token tests cannot verify that spans reach the host. The clean seam is
`WasiOtelCtx`:

```rust
pub trait WasiOtelCtx: Debug + Send + Sync + 'static {
    fn export_traces(&self, request: ExportTraceServiceRequest) -> FutureResult<()>;
    fn export_metrics(&self, request: ExportMetricsServiceRequest) -> FutureResult<()>;
}
```

`OtelDefault` logs counts and discards. For tests, implement a **capturing
backend** (mirroring the model test's `Recording` / `StubBackend`):

```rust
#[derive(Debug, Clone, Default)]
struct CapturingOtel {
    traces: Arc<Mutex<Vec<ExportTraceServiceRequest>>>,
    metrics: Arc<Mutex<Vec<ExportMetricsServiceRequest>>>,
}

impl WasiOtelCtx for CapturingOtel {
    fn export_traces(&self, request: ExportTraceServiceRequest) -> FutureResult<()> {
        self.traces.lock().unwrap().push(request);
        async { Ok(()) }.boxed()
    }
    // export_metrics similarly
}
```

Drive a guest that uses `#[instrument]` and assert the captured export contains
expected span names (`http_guest_handle`, `handler`) and metric counters.

**Driving the guest:** the otel example exports `wasi:http/handler.handle`, not a
simple `run` export. Two options:

| Approach | Pros | Cons |
|----------|------|------|
| **Tiny non-HTTP guest** with a `run` export that calls an `#[instrument]`'d function | Same `call_async` pattern as `replay.rs`; isolates the macro from HTTP | New example/guest to maintain |
| **Drive through HTTP in-process** on an ephemeral port (`HTTP_ADDR=127.0.0.1:0`) | Faithful to `examples/otel` | Heavier; server task lifecycle |

Recommend the tiny guest for macro-focused tests; keep the HTTP path for a
broader otel integration test later.

### 3.4 Table-driven examples smoke test

Generalize the otel manual workflow: a single integration test (or a small suite)
that, per example:

1. Builds the guest (or relies on `build-guests`).
2. Boots the runtime on an ephemeral port.
3. Issues one request (HTTP curl equivalent, or `call_async` for non-HTTP guests).
4. Asserts status 200 and expected response body / side effect.

This converts the `examples/*` surface from "run manually" into CI coverage
without duplicating the full per-interface test harness for every crate.

## 4. Suggested priority

| Order | Work | Effort | Impact |
|-------|------|--------|--------|
| 1 | Verify/fix CI to build guests before `nextest` | Low | Existing tests stop skipping silently |
| 2 | `body()` unit tests + `trybuild` UI tests for `wasi-otel-attr` | Low | Fast macro regression coverage |
| 3 | `CapturingOtel` backend + behavioural test with a tiny instrumented guest | Medium | Replaces manual `examples/otel` run for macro validation |
| 4 | Extract shared test-support crate | Medium | Unblocks tests for all WASI interfaces |
| 5 | Table-driven examples smoke test | Medium–High | Broad CI coverage across examples |

## 5. Non-goals

- Replacing unit tests inside individual host/guest crates where they already
  exist.
- Running a real OpenTelemetry Collector in CI (the capturing backend and
  `OtelDefault` are sufficient for integration tests).
- Testing guest compilation in CI without the `wasm32-wasip2` target (the
  `build-guests` task already requires it, which matches `rust-toolchain.toml`).

## 6. Open questions

1. Should `clean` remain on the default `test` path, or move to an explicit
   `test-clean` task? Cleaning before every local test run is slow and is the
   root cause of the skip behaviour.
2. Should `test-support` be a published crate or a path-only dev-dependency?
3. For HTTP-driven tests, should the host expose a programmatic in-process
   request API (bypassing TCP) to simplify test setup?
