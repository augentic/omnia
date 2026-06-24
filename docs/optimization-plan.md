# Runtime Optimization Plan

Plan for tuning the Omnia runtime to better exploit Wasmtime's pooling allocator and the per-request instantiation model used by `wasmtime serve`.

> Status: Plan A and Plan B Steps 1 & 2 are **implemented**. MPK (Plan B Step 2) is gated behind the opt-in `mpk` Cargo feature and is off by default. Plan B's remaining ideas and the "Other improvements" section below are still proposals.

## Background

Omnia already runs on the model that `wasmtime serve` uses: a single pre-instantiated component (`InstancePre`) plus a **fresh instance per request**, backed by the **pooling allocator**. The pooling allocator pre-reserves a fixed pool of slots (linear memories, tables, async stacks) and recycles them, so per-request instantiation is cheap.

**N.B. We should not pool live instances/stores.** Reusing a `Store` + `Instance` across requests breaks the per-request isolation guarantee of the component model. The correct approach (and the one we already follow) is *instantiate-per-request + pooling allocator*. Everything below keeps that model.

### Current state

- Pooling enabled in `crates/omnia/src/create.rs`:
  ```rust
  let mut pool = PoolingAllocationConfig::new();
  pool.total_component_instances(options.pool_max_instances)
      .total_core_instances(options.pool_max_instances)
      .total_memories(options.pool_max_instances)
      .total_tables(options.pool_max_instances)
      .total_stacks(options.pool_max_instances)
      .max_memory_size(options.pool_max_memory_bytes.unwrap_or(options.max_memory_bytes));
  ```
- Copy-on-write heap images (`Config::memory_init_cow`) are left at the default (`true`) — good, this is what makes per-request instantiation cheap.
- Per-request hot paths re-derive binding indices on every call:
  - `crates/wasi-http/src/host/server.rs` rebuilds `ServiceIndices::new(instance_pre)` per request.
  - `crates/wasi-messaging/src/host/server.rs` and `crates/wasi-websocket/src/host/server.rs` build their bindings (`MessagingRequestReply::new`, `Duplex::new`) per request.

### Known issues / opportunities

1. **Coupled pool totals (latent bug).** `total_memories` and `total_tables` are pinned to `pool_max_instances` (the *component-instance* count). A single component instance can transitively contain more than one core instance/memory/table, so a multi-memory guest would exhaust the memory pool before reaching the advertised instance count and instantiation would fail under load. `wasmtime serve` keeps these totals independent and constrains *per-component* counts instead.
2. **No residency tuning.** `linear_memory_keep_resident` / `table_keep_resident` / `async_stack_keep_resident` are unset, so every slot reuse pays for page decommit/zeroing. This is the biggest latency lever.
3. **No** `max_unused_warm_slots` **control.** Defaults to 100; worth making explicit and tunable.
4. **Per-request binding lookup** in the hot path (HTTP/messaging/websocket).
5. **No MPK /** `pagemap_scan` **/** `decommit_batch_size` — advanced levers `wasmtime serve` exposes.

---

## Plan A — Simple (low risk, high value)

Goal: remove obvious per-request waste and add the single most impactful pooling lever, without expanding the configuration surface much.

### Step 1 — Hoist per-request binding lookups out of the hot path

The expensive part of binding setup (`ServiceIndices::new(instance_pre)`, `*Pre`-style index computation) only depends on the stable `InstancePre`, not on the per-request `Store`/`Instance`. Compute it **once** and reuse it.

- `crates/wasi-http/src/host/server.rs`: build `ServiceIndices` once in `serve()` (before the accept loop) or cache it on `Handler`; keep only `indices.load(&mut store, &instance)` per request.
- `crates/wasi-messaging/src/host/server.rs` and `crates/wasi-websocket/src/host/server.rs`: same pattern — pre-compute the index/`*Pre` portion once per `Handler`, do only the per-instance `load`/`new` bind per request.

Zero behavioural change; removes export-name lookups from every request.

### Step 2 — Add residency tuning + decouple pool totals

In `crates/omnia/src/create.rs`, within the existing `if options.pooling` block:

```rust
let mut pool = PoolingAllocationConfig::new();
pool.total_component_instances(options.pool_max_instances)
    .total_core_instances(options.pool_max_instances)
    .total_memories(options.pool_max_instances)
    .total_tables(options.pool_max_instances)
    .total_stacks(options.pool_max_instances)
    .max_memory_size(options.pool_max_memory_bytes.unwrap_or(options.max_memory_bytes))
    // keep memory/tables/stacks resident to skip decommit/zeroing on reuse
    .linear_memory_keep_resident(options.pool_memory_keep_resident)
    .table_keep_resident(options.pool_table_keep_resident)
    .async_stack_keep_resident(options.pool_async_stack_keep_resident)
    .max_unused_warm_slots(options.pool_max_unused_warm_slots);
```

Add the corresponding `*` env-backed fields to `RuntimeOptions` in `crates/omnia/src/config.rs`, with conservative defaults so behaviour is unchanged until opted into:


| Field                            | Env var                          | Default | Notes                                 |
| -------------------------------- | -------------------------------- | ------- | ------------------------------------- |
| `pool_memory_keep_resident`      | `POOL_MEMORY_KEEP_RESIDENT`      | `0`     | bytes kept resident per linear memory |
| `pool_table_keep_resident`       | `POOL_TABLE_KEEP_RESIDENT`       | `0`     | bytes kept resident per table         |
| `pool_async_stack_keep_resident` | `POOL_ASYNC_STACK_KEEP_RESIDENT` | `0`     | bytes kept resident per async stack   |
| `pool_max_unused_warm_slots`     | `POOL_MAX_UNUSED_WARM_SLOTS`     | `100`   | matches Wasmtime default              |


These are all **runtime-only** settings — they do not affect the compiled artifact, so they are safe to add without touching the compile/run parity contract documented in `config.rs` (i.e. they won't break `Component::deserialize_file`).

> Note on the latent bug: the full decoupling (independent totals + per-component / per-module limits) landed in Plan B, Step 1 below.

**Validation:** benchmark a representative guest under load; raise keep-resident values and observe the RSS-vs-latency trade-off.

---

## Plan B — Comprehensive (matches `wasmtime serve`, more config surface)

Goal: bring the pooling configuration to parity with `wasmtime serve`, fix the latent totals bug, and make the whole knob set environment-driven and observable.

### Step 1 — Correct, fully-parameterised pooling configuration — **implemented**

Implemented in the `build_config` helper in [`crates/omnia/src/create.rs`](../crates/omnia/src/create.rs) and the new fields in [`crates/omnia/src/options.rs`](../crates/omnia/src/options.rs), mirroring the `wasmtime serve` config builder:

- `total_*` are now independent of the component-instance count — `total_core_instances`, `total_memories`, `total_tables`, and `total_stacks` each have their own knob — fixing the latent exhaustion bug.
- Per-component limits added: `max_core_instances_per_component`, `max_memories_per_component`, `max_tables_per_component`.
- Per-module limits added: `max_memories_per_module`, `max_tables_per_module`.
- Instance-size overrides added: `max_core_instance_size`, `max_component_instance_size`.
- `decommit_batch_size` added (batch decommits to amortise syscalls).
- `pagemap_scan` exposed (Linux `PAGEMAP_SCAN` ioctl; the default `no`/`auto` fall back cleanly off-Linux).

Structural limits and sizes are `Option`-typed and only applied when explicitly set, so an unset value preserves the Wasmtime default exactly. Cross-field invariants (e.g. a per-module count exceeding its pool total) are validated at start-up by `RuntimeOptions::validate`.

New `POOL_*` env vars (all runtime-only; `RuntimeOptions::requirements()` prints the authoritative list with defaults):

| Field                                   | Env var                                 | Default                             |
| --------------------------------------- | --------------------------------------- | ----------------------------------- |
| `pool_total_core_instances`             | `POOL_TOTAL_CORE_INSTANCES`             | `1000`                              |
| `pool_total_memories`                   | `POOL_TOTAL_MEMORIES`                   | `1000`                              |
| `pool_total_tables`                     | `POOL_TOTAL_TABLES`                     | `1000`                              |
| `pool_total_stacks`                     | `POOL_TOTAL_STACKS`                     | `1000`                              |
| `pool_max_core_instances_per_component` | `POOL_MAX_CORE_INSTANCES_PER_COMPONENT` | unset (Wasmtime default: unlimited) |
| `pool_max_memories_per_component`       | `POOL_MAX_MEMORIES_PER_COMPONENT`       | unset (Wasmtime default: unlimited) |
| `pool_max_tables_per_component`         | `POOL_MAX_TABLES_PER_COMPONENT`         | unset (Wasmtime default: unlimited) |
| `pool_max_memories_per_module`          | `POOL_MAX_MEMORIES_PER_MODULE`          | unset (Wasmtime default: 1)         |
| `pool_max_tables_per_module`            | `POOL_MAX_TABLES_PER_MODULE`            | unset (Wasmtime default: 1)         |
| `pool_max_core_instance_size`           | `POOL_MAX_CORE_INSTANCE_SIZE`           | unset (Wasmtime default: 1 `MiB`)   |
| `pool_max_component_instance_size`      | `POOL_MAX_COMPONENT_INSTANCE_SIZE`      | unset (Wasmtime default: 1 `MiB`)   |
| `pool_decommit_batch_size`              | `POOL_DECOMMIT_BATCH_SIZE`              | unset (Wasmtime default: 1)         |
| `pool_pagemap_scan`                     | `POOL_PAGEMAP_SCAN`                     | `no` (one of `auto`/`yes`/`no`)     |

### Step 2 — Optional MPK + observability + benchmarking harness — **implemented**

- **Memory protection keys (MPK):** gated behind the opt-in `mpk` Cargo feature on the `omnia` crate (which enables `wasmtime/memory-protection-keys`), off by default. Controlled by `POOL_MEMORY_PROTECTION_KEYS` (`auto`/`yes`/`no`, default `no`) and `POOL_MAX_MEMORY_PROTECTION_KEYS`, applied by `apply_mpk` in [`crates/omnia/src/create.rs`](../crates/omnia/src/create.rs). MPK only functions on Linux/x86_64; elsewhere it compiles but is inert (`auto` keeps guard regions). `POOL_MEMORY_PROTECTION_KEYS=yes` without the `mpk` feature is rejected at start-up. MPK is most effective with a reduced `POOL_MAX_MEMORY_BYTES` so the smaller memories can be striped.
- **Observability:** a background sampler ([`crates/omnia/src/metrics.rs`](../crates/omnia/src/metrics.rs), spawned from the runtime macro's `start()` in [`crates/runtime-macro/src/expand.rs`](../crates/runtime-macro/src/expand.rs)) reads `Engine::pooling_allocator_metrics()` every `POOL_METRICS_INTERVAL_MS` (default 5000ms; `0` disables) and emits pool-occupancy and warm-slot gauges. `State::instantiate` (in [`crates/omnia/src/traits.rs`](../crates/omnia/src/traits.rs), used by the HTTP/messaging/websocket servers) records an `instantiation_duration_us` histogram and a `pool_instantiation_errors` counter. Everything flows through the existing `tracing` -> OpenTelemetry metrics layer in `crates/otel`.
- **Benchmark harness:** a self-contained Rust load-test ([`examples/bench/http_bench.rs`](../examples/bench/http_bench.rs), no external load tools) drives concurrent keep-alive requests at the `http` example guest and reports throughput plus p50/p90/p99/p99.9 latency. See [`examples/bench/README.md`](../examples/bench/README.md) for how to build the guest + host + harness, run the host, drive load, and sample host RSS. Tunable via `BENCH_*` env vars (concurrency, duration, address, timeout) and the `POOL_*` knobs on the host.

---

## Other improvements worth considering

- **Make CoW explicit.** `memory_init_cow` defaults to `true`, but setting it explicitly in `create.rs` documents intent and guards against an accidental future default change. (Runtime-only; no artifact impact.)
- **Tune memory reservation/guard sizes.** `memory_reservation`, `memory_reservation_for_growth`, and `memory_guard_size` interact with pooling density and signal-based trap handling; defaults are reasonable but worth reviewing for our guest profile.
- `async_stack_zeroing` **as defense-in-depth.** Off by default for performance; consider enabling for untrusted guests, accepting the cost.
- **GC defaults (46.0.0).** The default collector is now the copying collector. If any guests use the component-model GC / reference types, validate behaviour and consider tuning `total_gc_heaps` and GC heap reservation knobs.
- **Security/maintenance discipline.** Enabling pooling + CoW is exactly the configuration historical Wasmtime advisories target (e.g. `GHSA-wh6w-3828-g9qf`, leaks fixed in 43.0.1 / 44.0.3 / 45.0.2). The existing comment in `Cargo.toml` already flags staying current on `46.0.x` patch releases — this matters more once we lean harder on the pool.
- **Engine sharing.** Confirm a single `Engine` is shared across all servers (it is, via the cloned `Context`/`InstancePre`). Keep it that way — the pooling allocator's pool is per-`Engine`.

---

## Suggested sequencing

1. Plan A, Step 1 (hoist binding lookups) — **done**; no new config.
2. Plan A, Step 2 (residency + warm slots) — **done**; biggest latency win.
3. Plan B, Step 1 (correct/parameterise pooling) — **done**; fixes the latent totals bug.
4. Plan B, Step 2 (MPK + observability + harness) — **done** (MPK behind the opt-in `mpk` feature). Use the [`http-bench` harness](../examples/bench/README.md) plus the new pool gauges to drive further residency/MPK tuning.

## Open questions

- Target deployment density: how many concurrent guest instances per host do we size the pool for?
- Are guests trusted or untrusted? (Drives `async_stack_zeroing` and how aggressively we reuse warm slots.)
- Which platforms must we support? (MPK and `pagemap_scan` are Linux-specific.)
- Do any guests use multiple memories/tables or component-model GC? (Drives the per-component limits and GC tuning.)

