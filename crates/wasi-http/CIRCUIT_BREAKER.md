# Circuit Breaker — Simplification & Placement Notes

Findings from a review of `crates/wasi-http/src/host/circuit_breaker.rs` and its
call-sites in `outbound.rs` / `default_impl.rs`. Two related questions:

1. Can the current implementation be simplified?
2. If we didn't implement it here, where else could it live?

---

## Part 1 — Simplifying the Current Implementation

The current breaker is ~840 lines (incl. tests): a three-state machine with a
fault window, a `BucketRegistry`, separate `probe_count` / `success_count`
counters, and internal `CheckOutcome` / `TripCause` enums used to defer logging
outside the mutex. Possible cuts, ranked by payoff:

### 1. Collapse `HalfOpen` to a single probe

Today `HalfOpen` tracks `probe_count` (admission cap) **and** `success_count`
(closes when ≥ `recovery_threshold`). The classic Hystrix/Polly pattern admits
one probe: success → `Closed`, failure → `Open`.

Removes: `recovery_threshold`, `probe_count`, `success_count`,
`HTTP_CB_RECOVERY_THRESHOLD`, and ~4 lifecycle tests.

### 2. Drop the fault window; reset on success instead

The fault window stops "1 timeout per hour for 10 hours" from tripping. A
simpler equivalent: **reset `fault_count` to 0 on every success while
`Closed`**. You only trip on a consecutive burst, no `Instant` arithmetic, no
`fault_window_start`, no `HTTP_CB_FAULT_WINDOW_MS`, no boundary-edge test.

`record_success` is currently a no-op in `Closed` — adding the reset there
deletes the entire window mechanism (~25 lines + tests + 1 env var).

### 3. Drop `BucketRegistry`; use a single global breaker

The bucket abstraction costs a `HashMap`, `Arc<CircuitBreaker>`,
`ResolvedBreaker`, an "unknown bucket" error path, the `x-omnia-upstream`
header round-trip, ~6 dedicated tests, and a benchmark group.

If real deployments use 1–3 fixed upstreams, hard-code a single
`Arc<CircuitBreaker>` in `ResilienceConfig` and delete the registry. ~80 lines
gone, plus the WASI boundary header.

### 4. Inline `CheckOutcome` / `TripCause` enums

They exist only to defer `tracing::*!` until after the `parking_lot::Mutex` is
released. `parking_lot` critical sections are very short and `tracing` macros
are synchronous + cheap — logging inside the lock is fine. Inlining removes
both enums and the post-match dispatch (~50 lines).

### 5. Drop the test-only `*_at` parametric API

Three pairs of `check`/`check_at`, `record_failure`/`record_failure_at`, etc.
Either inject a `Clock` trait (more abstraction) or keep just the parametric
form and always pass `Instant::now()` at the call site.

### 6. Drop `BreakerConfig::validate`

Switch to `NonZeroU32` + `Duration` in the config struct; invalid values
become unconstructible. Removes 4 validation tests.

### 7. Reach for an off-the-shelf crate

`failsafe-rs` (or a `tower` CB layer) gives you closed/open/half-open in ~10
LOC of integration. The current breaker isn't exotic enough to justify a
custom impl once buckets go away.

### Recommended minimum

1, 2, and 4 alone take the file from ~840 → ~400 lines (mostly tests), remove
two env vars, and the state machine fits on a screen. Buckets stay — they
add capability rather than just knobs.

---

## Part 2 — Where Else a Circuit Breaker Could Live

Roughly in increasing distance from the application code.

### 1. Runtime/platform layer (sidecar or proxy)

Envoy/Linkerd/Istio do circuit breaking + outlier detection as first-class
features. Configure per-cluster and every outbound HTTP call is covered.

- **Pros:** zero application code, operator-tuned, language-agnostic.
- **Cons:** HTTP only — doesn't help `wasi-sql`, `wasi-messaging`,
  `wasi-blobstore`. Assumes the operator runs a mesh.

Right answer if Omnia is always deployed under an opinionated platform.

### 2. `reqwest` / `tower` middleware (still in `wasi-http`)

Wrap `reqwest::Client` in a `tower::Service`; compose `tower::retry`,
`tower::timeout`, `tower::load_shed`, and a CB layer (e.g. `failsafe`).

- **Pros:** stays in `wasi-http`, deletes the hand-rolled state machine.
- **Cons:** pulls in the `tower` ecosystem; stacked middlewares can be opaque.

### 3. Guest-side, via `omnia-sdk`

Move the breaker into the WASM component itself, alongside `OutboundPolicy`.

- **Pros:** portable across host implementations, per-feature isolation is
  automatic, host stays simpler.
- **Cons:** if instances are request-scoped the breaker has no memory between
  calls and is useless; WASM crossing cost per `check` / `record`; can't share
  trip state across instances without going through the host.

Only viable for long-lived worker guests.

### 4. Generic `Resilient<B: Backend>` adapter in the `omnia` crate

One breaker wrapper any `Backend` can opt into.

- **Pros:** one implementation, reusable across SQL / messaging / blobstore /
  HTTP.
- **Cons:** each backend has different fault semantics, so the wrapper needs a
  `BreakerFault` trait — not obviously simpler than per-interface breakers.

### 5. Per-interface (today's pattern, expanded)

Add breakers to `wasi-sql`, `wasi-messaging`, `wasi-blobstore`,
`wasi-keyvalue` as needed.

- **Pros:** each tuned to its protocol's fault model.
- **Cons:** the drivers (`sqlx`, `async-nats`, `aws-sdk-s3`) already have
  retry/timeout knobs; an extra CB on top is sometimes redundant.

### 6. Skip the breaker; use a simpler primitive

A CB does three jobs: fast-fail when sick, stop piling on, test recovery.
Most of that comes free from:

- **Concurrency limiter** — `tokio::sync::Semaphore` per upstream caps
  in-flight requests. When the upstream slows down, new requests reject
  immediately. ~15 LOC, no state machine.
- **Strict total-budget timeout** — already present.
- **Rate limiter / token bucket** — bounds per-upstream load.

A semaphore + the existing timeout covers ~80% of what the breaker actually
does for you.

---

## Recommendation for this codebase

Given that:

- `wasi-http` is the only crate that has rolled its own breaker,
- Omnia is intended to be deployable as a standalone binary (no sidecar
  guarantee),
- but most real production users *will* be behind a proxy or mesh,

A two-step path is sensible:

1. **Replace the breaker with a `tokio::sync::Semaphore`** keyed by
   `upstream` — `HTTP_MAX_INFLIGHT_PER_UPSTREAM=N`. Handles "don't pile on a
   sick host" without state machines or fault windows.
2. **Document** in the `wasi-http` README that real circuit breaking is the
   responsibility of an upstream proxy/mesh, and recommend an Envoy snippet.

If a real breaker turns out to be necessary after that, add it back as a
`tower` middleware on the `reqwest::Client` (option 2 above) — the whole
`circuit_breaker.rs` file goes away in favour of a maintained crate.
