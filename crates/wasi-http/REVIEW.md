# Circuit Breaker Review — Implementation Plan

Plan derived from three independent reviews (opus-4.7, composer-2.5, gpt-5.5) of `crates/wasi-http/src/host/circuit_breaker.rs` and its integration in `outbound.rs` / `default_impl.rs`.

**Current “feels like Omnia” scores:** 6.0–7.5 / 10 (consensus: functionally solid, stylistically off).

**Target after Section 1:** ~8.5 / 10.

**Primary files:**

| File | Role |
|------|------|
| `crates/wasi-http/src/host/circuit_breaker.rs` | State machine, registry, tests |
| `crates/wasi-http/src/host/outbound.rs` | Breaker check, fault classification, integration tests |
| `crates/wasi-http/src/host/default_impl.rs` | `ConnectOptions`, registry construction |

---

## 1. Highest-leverage fixes

These are consensus items across all three reviewers. Implement in order; each is independently valuable but later items assume earlier naming/API choices.

### 1.1 Unify configuration under `ConnectOptions` / `FromEnv`

**Problem:** Global CB settings load via `#[derive(FromEnv)]` on `ConnectOptions` (`default_impl.rs:46–72`), but per-bucket overrides read raw `std::env::var` inside `bucket_config_from_env` (`circuit_breaker.rs:294–321`). This is the only env-reading pattern of its kind in the monorepo. Invalid values silently fall back; tests cannot easily mock per-bucket config.

**Recommended approach (pick one):**

| Option | Effort | Notes |
|--------|--------|-------|
| **A. Drop per-bucket env overrides for v1** | Low | Simplest. `BucketRegistry::new` takes only `ConnectOptions` (or a `BreakerConfig` + bucket list). Document that all buckets share global thresholds until a v2 config surface exists. |
| **B. Centralize overrides in `ConnectOptions`** | Medium | Add e.g. `HTTP_CB_OVERRIDES` as a serde/JSON map, or document known bucket names as explicit `FromEnv` fields. Parse once in `connect_with`, pass a `HashMap<String, BreakerConfig>` into `BucketRegistry::new`. |

**Tasks:**

- [ ] Remove `bucket_config_from_env` and all `std::env::var` calls from `circuit_breaker.rs`.
- [ ] Construct per-bucket configs in `default_impl.rs` (or a small `cb_config.rs` helper next to `ConnectOptions`) before calling `BucketRegistry::new`.
- [ ] Invalid env values should fail at startup via `fromenv`, matching the rest of Omnia — not silently fall back.
- [ ] Add/update tests for config loading (unit test with constructed options; no env mutation in `circuit_breaker` tests).

### 1.2 Replace `Result<(), ()>` with an explicit decision type

**Problem:** `CircuitBreaker::check` returns `Result<(), ()>` (`circuit_breaker.rs:78–79`). Callers immediately discard the error and build their own string (`outbound.rs:92–96`). No other public Omnia API uses this pattern.

**Tasks:**

- [ ] Introduce `pub enum BreakerDecision { Allow, Reject }` (or `pub fn is_allowed(&self) -> bool` if distinction is never needed).
- [ ] Update `check` / `check_at` signatures and all call sites in `outbound.rs`.
- [ ] Update unit tests (`assert!(cb.check().is_ok())` → `assert_eq!(cb.check(), BreakerDecision::Allow)` or equivalent).

### 1.3 Rename states to conventional vocabulary

**Problem:** `State::{Off, On, HalfOn}` inverts industry convention (`Off` = closed = allowing traffic). Every reader and every `gauge.circuit_breaker_state = 0|1|2` lookup pays a translation tax.

**Tasks:**

- [ ] Rename to `Closed`, `Open`, `HalfOpen` (or `HalfOpen` / `Probe` if preferred).
- [ ] Update module-level docs (`circuit_breaker.rs:7–16`, `48–51`).
- [ ] Update tracing: prefer `state = "half_open"` string fields over magic-number gauges, or document the gauge mapping in module docs if gauges must stay numeric for dashboards.
- [ ] Rename test names and assertions (`transitions_to_on_after_threshold` → `transitions_to_open_after_threshold`, etc.).
- [ ] Grep the repo for `State::Off|On|HalfOn` and fix all references.

### 1.4 Validate `BreakerConfig` at construction

**Problem:** `switch_off_threshold = 0` makes `HalfOpen` reject every probe (`probe_count < 0` never true), so the breaker can never recover. Malformed durations/thresholds are accepted silently.

**Tasks:**

- [ ] Add `BreakerConfig::validate(&self) -> Result<(), ConfigError>` or `TryFrom<ConnectOptions>` that checks:
  - `switch_on_threshold >= 1`
  - `switch_off_threshold >= 1`
  - `reset_period > Duration::ZERO`
  - `fault_window > Duration::ZERO`
- [ ] Call validation in `default_impl.rs` when building resilience config (fail startup loudly).
- [ ] Add tests for invalid configs (expect error, not silent misbehavior).
- [ ] Document intentional semantics of `switch_on_threshold = 1` (trips on first fault).

### 1.5 Fix `record_success_at` to honor injected time

**Problem:** `record_success_at(&self, _now: Instant)` ignores `_now` and calls `Instant::now()` at line 150. Every other `*_at` method honors the passed clock, breaking deterministic test patterns.

**Tasks:**

- [ ] Use `now` for `fault_window_start` reset on recovery (line 150).
- [ ] Verify existing lifecycle tests still pass; add a test that recovery timestamp depends on injected `now`.

### 1.6 Correct observability for default-bucket fallback

**Problem:** `BucketRegistry::resolve` falls back to `"default"` for unknown upstream names (`circuit_breaker.rs:276–284`), but `outbound.rs:93–95` logs/errors using the *requested* upstream name. Error reads `circuit breaker open: nonexistent` when the `"default"` breaker tripped.

**Tasks:**

- [ ] Change `resolve` to return `(Arc<CircuitBreaker>, &'static str)` or a small `ResolvedBreaker { breaker, bucket: &str }` where `bucket` is the registry key actually used (`"default"` on fallback).
- [ ] Use resolved bucket name in tracing and `ErrorCode` messages in `outbound.rs`.
- [ ] Add an integration/unit test: unknown upstream + open default breaker → error mentions `"default"` (or documents intentional behavior if product decision differs).

### 1.7 Clarify and align fault-window semantics

**Problem:** Module doc says “rolling window” and “resets when no new fault arrives” (`circuit_breaker.rs:50–51`), but `record_failure_at` implements a **fixed window anchored at first fault** (`fault_window_start` reset only when elapsed > `fault_window`, strict `>` at line 181). gpt-5.5 flagged this as a doc/implementation mismatch; composer-2.5 noted the strict-`>` boundary.

**Decision required (pick one):**

| Option | Action |
|--------|--------|
| **Keep fixed window** | Rewrite module docs and field comments to say “fixed fault window.” Add a one-line note on strict-`>` boundary behavior. |
| **Implement true rolling window** | Track failure timestamps (e.g. `VecDeque<Instant>` or ring buffer) — higher complexity; only if product requires it. |

**Tasks:**

- [ ] Apply chosen option.
- [ ] Ensure `fault_window_resets_count` and `fault_window_boundary` tests still describe actual behavior.

### 1.8 Document breaker fault vs success semantics

**Problem:** `is_breaker_fault` in `outbound.rs:124–128` treats 4xx and most non-timeout reqwest errors as *success* for the breaker. Reasonable for an availability breaker, but undocumented. composer-2.5 and opus-4.7 both flagged this.

**Tasks:**

- [ ] Add module-level doc in `circuit_breaker.rs` or a short comment block in `outbound.rs` listing what counts as fault vs success:
  - **Fault:** 5xx, connect error, timeout error; 429 explicitly excluded.
  - **Success (for recovery):** any completed response that is not a fault, including 4xx.
- [ ] Optionally extend `outbound.rs` integration tests to assert 404 does not trip / does count toward HalfOpen recovery.

### 1.9 Simplify lock scope in the state machine

**Problem:** Redundant `drop(inner)` calls (`circuit_breaker.rs:94–99, 105–110, 190, 199`), `transition: Option<State>` tuple, and `#[allow(clippy::if_then_some_else_none)]` on `record_failure_at` (line 175) add noise without benefit — the block already drops the guard.

**Tasks:**

- [ ] Remove all explicit `drop(inner)` where the guard goes out of scope immediately after.
- [ ] Refactor `record_failure_at` to satisfy clippy without `#[allow]` (e.g. `bool::then`, early returns).
- [ ] Optional: extract a private `fn apply_transition(&self, transition: Option<State>, allowed: bool)` for tracing to shrink `check_at`.
- [ ] Optional (gpt-5.5): internal event enum (`Allowed`, `Rejected`, `EnteredHalfOpen`, `Tripped`) to unify tracing + return value — only if it reduces line count.

**Acceptance for Section 1:**

```bash
cargo test -p omnia-wasi-http circuit_breaker --all-features
cargo test -p omnia-wasi-http outbound --all-features   # integration tests
cargo clippy -p omnia-wasi-http --all-features
```

All existing breaker behavior tests should pass (modulo renames). New tests cover config validation, resolved bucket naming, and `_now` in `record_success_at`.

---

## 2. Nits by reviewer

Lower priority or stylistic. Tackle after Section 1, or inline when touching the same code.

### 2.1 opus-4.7

| # | Item | Location | Suggested fix |
|---|------|----------|---------------|
| O1 | Startup `warn!` when no buckets configured | `circuit_breaker.rs:263–267` | Demote to `debug!` or remove — empty `HTTP_CB_BUCKETS` is the default, not an anomaly. |
| O2 | `switch_off_threshold` overloaded | `Inner::probe_count` vs `success_count` | Add a doc comment on `BreakerConfig::switch_off_threshold` explaining it caps in-flight probes *and* required successes to close. Or split into two fields in a follow-up. |
| O3 | Magic-number tracing gauges | `circuit_breaker.rs:119, 163, 210` | Use string state labels in tracing fields, or document gauge mapping in module docs. |
| O4 | `bucket_config_from_env` verbosity | `circuit_breaker.rs:294–321` | If keeping env reads temporarily, compress with a shared `env_or<T: FromStr>(key, fallback)` helper. **Superseded by 1.1 if overrides are removed.** |
| O5 | `Inner::opened_at` sentinel | `circuit_breaker.rs:45, 69` | Consider `Option<Instant>` — only set when entering `Open`. Low priority. |
| O6 | `CircuitBreaker::name: String` | `circuit_breaker.rs:54` | `Arc<str>` or rely on registry key only. Low priority unless hot path. |
| O7 | `#[cfg(test)] pub const fn default_breaker` | `circuit_breaker.rs:287–290` | Drop meaningless `const`; plain `fn` is fine. |
| O8 | Per-bucket overrides as hidden config surface | design | If not dropped in 1.1, document in crate README or `ConnectOptions` rustdoc with example env vars. |

### 2.2 composer-2.5

| # | Item | Location | Suggested fix |
|---|------|----------|---------------|
| C1 | Non-timeout/connect `reqwest::Error` not a fault | `outbound.rs:126` | Document (see 1.8) or broaden `is_breaker_fault` to treat all transport errors as faults. |
| C2 | Unknown upstreams share one default breaker | `circuit_breaker.rs:276–284` | Document in module docs. Optional v2: lazy per-upstream breaker creation on first sight. |
| C3 | `switch_off_threshold = 0` edge case | config | Covered by 1.4 validation. |
| C4 | `per_bucket_override_applied` doesn't test env | `circuit_breaker.rs:657–673` | After 1.1, replace with test that passes explicit per-bucket config into `BucketRegistry::new`. Remove the “We test the non-env path” comment. |
| C5 | Verbose `check_at` HalfOpen branch | `circuit_breaker.rs:102–112` | Optional private `transition` helper (may overlap with 1.9). |
| C6 | Test helper style | `circuit_breaker.rs:738` | Use `Arc::clone(reg.default_breaker())` instead of fully qualified path. |
| C7 | File size (~750 lines, ~430 tests) | `circuit_breaker.rs` | Optional: move `mod tests` to `circuit_breaker/tests.rs` or `tests/circuit_breaker.rs` if author prefers leaner production module. Matches `retry.rs` density today — not blocking. |
| C8 | `std::ptr::eq` vs `Arc::ptr_eq` | `circuit_breaker.rs:713–720` | Prefer `Arc::ptr_eq` consistently (already used elsewhere in same file). |

### 2.3 gpt-5.5

| # | Item | Location | Suggested fix |
|---|------|----------|---------------|
| G1 | `DashMap` for immutable registry | `circuit_breaker.rs:238` | Replace with `HashMap<String, Arc<CircuitBreaker>>` — buckets are built once at startup, only read afterward. Remove `dashmap` dep from `wasi-http/Cargo.toml` if unused elsewhere. |
| G2 | Fixed vs rolling window | `circuit_breaker.rs:50–51, 181` | Covered by 1.7. |
| G3 | Error misidentifies breaker on fallback | `outbound.rs:93–95` | Covered by 1.6. |
| G4 | Repetitive single-case unit tests | `circuit_breaker.rs:323–748` | Optional refactor: table-driven tests for threshold/window/probe scenarios. Reduces scan noise; keep integration tests in `outbound.rs` as-is. |
| G5 | Config split / heavier than nearby modules | design | Covered by 1.1 and 1.9. |

---

## Suggested implementation order

```
1.1 Config unification          ─┐
1.4 Config validation            ├── startup correctness
1.5 record_success_at _now fix   ─┘
1.2 BreakerDecision API          ─── public surface
1.3 State rename                 ─── readability (touch tests heavily)
1.6 Resolved bucket observability
1.7 Fault window docs/impl
1.8 Fault vs success docs + tests
1.9 State machine cleanup        ─── clippy + drop(inner)
Section 2 nits                   ─── as time permits
```

## Out of scope (unless explicitly requested)

- True sliding/rolling fault window implementation (vs doc fix).
- Splitting `switch_off_threshold` into separate probe-cap and success-threshold knobs.
- Lazy dynamic bucket creation for undeclared upstream names.
- Manual force-open/close admin API.
- Dedicated `monotonic_counter.circuit_breaker_rejections` metric changes (already present; only adjust if tracing shape changes).

---

## Reviewer score reference

| Reviewer | Score | One-line summary |
|----------|-------|------------------|
| opus-4.7 | 6.5 / 10 | Breaks `FromEnv`, weak `Result<(), ()>`, inverted names, `_now` bug |
| composer-2.5 | 7.5 / 10 | Production-ready tests/integration; naming and config split feel bespoke |
| gpt-5.5 | 6.0 / 10 | Tests pass (29/29); fixed-window doc mismatch, `DashMap` overkill, verbose SM |

**Consensus praise (preserve):** `parking_lot::Mutex`, module layout beside `retry.rs`, opt-in `ResilienceConfig`, injectable `*_at(now)` testing, wiremock integration tests in `outbound.rs`, tracing metrics pattern.
