# Design: Integration Testing in Omnia

> Status: **Adopted / in progress.** The CI/build fix (§3.1) and the shared
> test crate (§3.2, shipped as `crates/testkit`) have landed, along with an
> in-process HTTP driver and per-interface seam tests. The testing policy this
> RFC implies is codified in §7 and in `AGENTS.md`.

## 1. The problem

Omnia is a WASM (WASI) component runtime. Most behaviour spans a guest `.wasm` component, the host runtime, and one or more WASI host backends. Unit tests on individual crates cannot exercise that seam.

Today, validation falls into two buckets:

1. **Automated integration tests** — only two exist: - `crates/omnia/tests/guest_link.rs` — host-mediated dynamic linking (`examples/guest-link`) - `crates/wasi-model/tests/replay.rs` — replay and `resolve` dispatch (`examples/model`)
2. **Manual example runs** — everything else (`otel`, `http`, `keyvalue`, `sql`, `blobstore`, …) is exercised by building a guest, starting the example host, and inspecting logs or curling an endpoint.

The `wasi-otel-attr` macro is a concrete example of the gap. The macro lives in `crates/wasi-otel-attr` with **zero** automated tests. To verify that `#[omnia_wasi_otel::instrument]` expands correctly and that instrumented spans reach the host, a developer must manually run `examples/otel`:

```bash
cargo build --example otel-wasm --target wasm32-wasip2
cargo run --example otel -- run ./target/wasm32-wasip2/debug/examples/otel_wasm.wasm
curl -d '{"text":"hello"}' http://localhost:8080
```

That pattern does not scale across 15+ WASI interfaces and proc-macro crates.

## 2. Current state

### 2.1 The existing integration-test pattern

Both `guest_link.rs` and `replay.rs` follow the same shape:

1. Locate a pre-built guest under `target/wasm32-wasip2/{debug,release}/examples/<name>_wasm.wasm`.
2. Write a temporary manifest with absolute paths to the guest(s).
3. Hand-roll a `TestCtx` / `TestRuntime` implementing the required host views.
4. Call `create_from_manifest`, link host interfaces, and drive guest exports via `call_async`.

The model test goes further: it swaps in a **stub backend** to assert host-side effects (replayed answer matches, `resolve` dispatches to a fresh `shelf` instance per call).

When the guest `.wasm` is absent, both tests **skip** (print instructions and return `Ok(())`) rather than fail. That is intentional — `cargo make test` and `cargo make ci` run `clean` before `nextest`, which removes any previously built guests.

### 2.2 CI gap

`Makefile.toml` wires `test` and `ci` through `clean`:

```toml
[tasks.test]
dependencies = ["clean"]
args = ["nextest", "run", "--all", "--all-features", "--no-tests=pass"]
```

There is **no** task that builds example guests for `wasm32-wasip2` before tests run. The only references to that target outside config are the skip messages in the two integration tests. Unless CI (`.github/workflows/ci.yaml`, which delegates to `augentic/.github/...@main`) builds guests after clean, the existing integration tests pass vacuously by skipping.

### 2.3 Proc-macro testing gap

`crates/wasi-otel-attr` is a proc-macro crate with no `[dev-dependencies]` and no test infrastructure (`trybuild`, `macrotest`, `insta`) anywhere in the repo. The same applies to `host-macros` and `guest-macro`.

## 3. Proposed improvements

### 3.1 Fix the CI/build pipeline (highest leverage) — DONE

`Makefile.toml` now has a `build-guests` task that `test` depends on, and `clean`
has moved off the default test path into a separate `test-clean` task (resolving
open question 1). Integration tests execute rather than skip in CI.

Add a `build-guests` task to `Makefile.toml` that builds all `*-wasm` examples for `wasm32-wasip2`:

```bash
cargo build -p examples \
  --examples \
  --target wasm32-wasip2
```

Make `test` depend on `build-guests` **after** `clean` (or drop `clean` from the default test path and reserve it for a separate `test-clean` task). Verify the shared CI workflow runs the guest build before `nextest` so `guest_link` and `replay` actually execute rather than skip.

**Outcome:** existing integration tests become meaningful in CI with no new test code.

### 3.2 Extract shared test support — DONE (as `crates/testkit`)

Shipped as `crates/testkit` (package `omnia-testkit`, `publish = false`,
path-only dev-dependency — resolving open question 2). It provides `find_guest`
(the "fail in CI, skip locally" locator), `temp_manifest` (a self-cleaning temp
manifest), and an in-process `http` driver (resolving open question 3 in the
test crate rather than production — see §3.4). `guest_link.rs` and `replay.rs`
were ported onto it, dropping the duplicated `common/mod.rs` and the `#[path]`
hack that `replay.rs` used to share it.

`guest_link.rs` and `replay.rs` duplicate roughly 100 lines each:

- `target_dir()` — derive `target/` from the test executable path
- `guest_wasm()` — locate a built guest by file name
- Manifest writing to a temp file
- A generic store-counting `TestRuntime` / `TestCtx`

Factor these into a `crates/test-support` crate (or a shared module behind a dev-dependency) so each WASI crate's integration test is ~20 lines of interface-specific wiring. Lowering the per-interface cost is the single biggest lever for adding more integration tests.

### 3.3 Proc-macro tests for `wasi-otel-attr`

The macro has two distinct concerns, best tested at two tiers.

#### Tier 1 — expansion and arg parsing (fast, native, no WASM)

The interesting logic lives in `body()` and `Attributes` in `crates/wasi-otel-attr/src/lib.rs`, which operate on `proc_macro2` / `syn` tokens (not `proc_macro::TokenStream`). Call `body()` directly from a `#[cfg(test)] mod tests` in the crate:

| Input                  | Expected expansion                                                 |
| ---------------------- | ------------------------------------------------------------------ |
| Async fn, no args      | `Instrument::instrument(async move …, span!(INFO, fn_name)).await` |
| Sync fn, no args       | `span!(INFO, fn_name).in_scope(\|\| { … })`                        |
| `name = "custom"`      | Span name is `"custom"`                                            |
| `level = Level::DEBUG` | Level propagates into `span!`                                      |

For the full macro surface (including the `proc_macro` boundary and compile-time errors), add `trybuild` as a dev-dependency with a `tests/ui/` folder:

- **Pass cases:** named span, leveled span, async/sync functions
- **Fail case:** `#[instrument(bogus = 1)]` → `"unsupported property"`

This generalizes to `host-macros` and `guest-macro`.

#### Tier 2 — behavioural (does instrumentation export spans?)

Token tests cannot verify that spans reach the host. The clean seam is `WasiOtelCtx`:

```rust
pub trait WasiOtelCtx: Debug + Send + Sync + 'static {
    fn export_traces(&self, request: ExportTraceServiceRequest) -> FutureResult<()>;
    fn export_metrics(&self, request: ExportMetricsServiceRequest) -> FutureResult<()>;
}
```

`OtelDefault` logs counts and discards. For tests, implement a **capturing backend** (mirroring the model test's stub backends):

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

Drive a guest that uses `#[instrument]` and assert the captured export contains expected span names (`http_guest_handle`, `handler`) and metric counters.

**Driving the guest:** the otel example exports `wasi:http/handler.handle`, not a simple `run` export. Two options:

| Approach                                                                             | Pros                                                                   | Cons                           |
| ------------------------------------------------------------------------------------ | ---------------------------------------------------------------------- | ------------------------------ |
| **Tiny non-HTTP guest** with a `run` export that calls an `#[instrument]`'d function | Same `call_async` pattern as `replay.rs`; isolates the macro from HTTP | New example/guest to maintain  |
| **Drive through HTTP in-process** on an ephemeral port (`HTTP_ADDR=127.0.0.1:0`)     | Faithful to `examples/otel`                                            | Heavier; server task lifecycle |

Recommend the tiny guest for macro-focused tests; keep the HTTP path for a broader otel integration test later.

**Landed (HTTP path):** `crates/wasi-otel/tests/seam.rs` drives the instrumented
`otel` example guest through `omnia_testkit::http` and swaps `OtelDefault` for a
`CapturingOtel` that **counts** exported spans and metrics (rather than storing
whole requests). Metrics flush deterministically when the guest's telemetry guard
drops at the end of the handler, so they are the reliable assertion; spans ride a
separate sampled batch flush and are counted only for diagnostics.

### 3.4 Per-interface seam tests

Realized as one `tests/seam.rs` per WASI crate. Rather than booting a TCP server
on an ephemeral port (open question 3), `omnia_testkit::http::handle` drives the
guest's `wasi:http/handler` export **in-process** — it mirrors the runtime's HTTP
trigger server (`crates/wasi-http/src/host/server.rs`): resolve the guest by
request path, instantiate fresh, hand it a `wasi:http` request, and collect the
response — but skips the socket. Each seam test then:

1. Relies on `build-guests` for the pre-built guest (skips locally if absent).
2. Builds a single-guest runtime with the example's default backends.
3. Issues one request via `omnia_testkit::http` (or `call_async` for the two
   `wasi:cli/run` guests, `cli` and `model`).
4. Asserts the response and, where the effect is host-side, the backend state
   (e.g. a `wasi:keyvalue` write is read back off the shared backend; `wasi:otel`
   spans are asserted via a capturing backend).

This converts the `examples/*` surface from "run manually" into CI coverage.
Landed seam tests: `wasi:keyvalue`, `wasi:blobstore`, `wasi:vault`, `wasi:config`,
`wasi:docstore` (storage round-trips), `wasi:http` (echo), multi-guest HTTP
routing (`crates/wasi-http/tests/routing.rs`), `wasi:websocket`, `wasi:messaging`
(publish observed on a host subscription), `wasi:otel` (capturing backend), and
the MCP transport (`crates/wasi-http/tests/mcp.rs`, since MCP is a guest-side
library over `wasi:http`).

Three interfaces have no offline happy path and keep their native unit tests
instead of a seam test:

- `wasi:identity` — its default performs a real OAuth2 exchange.
- `wasi:sql` — SQLite with no auto-provisioned schema.
- `wasi:http` **outbound** (`examples/http-proxy`) — the guest calls real public
  origins, so its seam is the in-crate `wiremock` suite in
  `crates/wasi-http/src/host/default_impl.rs`, which already drives a real HTTP
  boundary against a mock server. It stays in place (it reaches private hooks, so
  it cannot move to `tests/` without leaking internals).

## 4. Suggested priority

| Order | Work                                                                      | Effort      | Impact                                                   |
| ----- | ------------------------------------------------------------------------- | ----------- | -------------------------------------------------------- |
| 1     | Verify/fix CI to build guests before `nextest`                            | Low         | Existing tests stop skipping silently                    |
| 2     | `body()` unit tests + `trybuild` UI tests for `wasi-otel-attr`            | Low         | Fast macro regression coverage                           |
| 3     | `CapturingOtel` backend + behavioural test with a tiny instrumented guest | Medium      | Replaces manual `examples/otel` run for macro validation |
| 4     | Extract shared test-support crate                                         | Medium      | Unblocks tests for all WASI interfaces                   |
| 5     | Table-driven examples smoke test                                          | Medium–High | Broad CI coverage across examples                        |

## 5. Non-goals

- Replacing unit tests inside individual host/guest crates where they already exist.
- Running a real OpenTelemetry Collector in CI (the capturing backend and `OtelDefault` are sufficient for integration tests).
- Testing guest compilation in CI without the `wasm32-wasip2` target (the `build-guests` task already requires it, which matches `rust-toolchain.toml`).

## 6. Open questions — resolved

1. **`clean` on the default `test` path?** No. `clean` moved to a dedicated
   `test-clean` task; `test` depends on `build-guests` instead, so guests are
   present and the integration tests execute rather than skip.
2. **`test-support` published or path-only?** Path-only, `publish = false`,
   shipped as `crates/testkit` (package `omnia-testkit`).
3. **In-process request API in the host?** Not in production. The in-process
   HTTP driver lives in `testkit` (`omnia_testkit::http`), mirroring the trigger
   server without a production surface change.

## 7. Testing policy

The policy this RFC leads to, now in force (see `AGENTS.md`):

- **Unit tests survive only for pure, deterministic logic**: parsers, codecs,
  filter/type translation, macro token expansion. Anything touching a WASI
  interface, a host backend, or dispatch is tested at the guest–host seam.
- **The seam test is the executable spec.** A new interface behaviour is a
  `tests/seam.rs` case (a behaviour contract like "put then get round-trips
  through the wasm boundary"), not a transliteration of an implementation detail.
- **Replace, then delete — never delete first.** A superseded unit-test module is
  removed in the same change as the seam test that covers it, with `cargo
  llvm-cov` before/after evidence on the affected host crate: delete only if
  line/branch coverage holds.
  - **Measured outcome (first pass).** Auditing the host crates against the new
    happy-path seams, most default-backend unit tests cover paths the seams do
    *not* reach — error mapping (`wasi-http` cert/URI/refused cases), isolation
    and delete (`wasi-vault`), inbound events (`wasi-websocket`), request/reply
    stubs (`wasi-messaging`) — or pure logic (bson filters, OData, manifest/route
    parsing, the dispatch selector/depth guard). Those are **retained**. Only two
    were genuinely superseded and removed: `wasi-docstore`'s `roundtrip_document`
    and `wasi-messaging`'s `pub_sub_delivers_to_subscriber`, whose exact host
    paths (`insert`/`get`, `send`/`subscribe`) the seam tests now drive.
    `cargo llvm-cov` confirmed coverage held — in fact rose, since the seam
    exercises more of the real path (`producer_impl.rs` 0% → 100%,
    `docstore/default_impl.rs` 30% → 54% lines).
- **Guest-side logic keeps native unit tests** where `llvm-cov` cannot instrument
  the guest `.wasm` (e.g. `crates/omnia-guest`).
- **Names identify, comments explain.** A test name is the scenario
  (`set_then_get`), not a restated expectation (`set_then_get_round_trips`); nuance
  goes in a `//` comment, consistent with the repo comment policy.
