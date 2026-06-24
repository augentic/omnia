# Runtime Optimization Plan

Plan for tuning the Omnia runtime to better exploit Wasmtime's pooling allocator and the per-request instantiation model used by `wasmtime serve`.

> Status: **draft for review**. Nothing here has been implemented yet.

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

> Note on the latent bug: a minimal first step can leave the totals as-is; the full decoupling (independent totals + per-component limits) is in Plan B, Step 1. Document the constraint until then.

**Validation:** benchmark a representative guest under load; raise keep-resident values and observe the RSS-vs-latency trade-off.

---

## Plan B — Comprehensive (matches `wasmtime serve`, more config surface)

Goal: bring the pooling configuration to parity with `wasmtime serve`, fix the latent totals bug, and make the whole knob set environment-driven and observable.

### Step 1 — Correct, fully-parameterised pooling configuration

Decouple totals and add per-component / per-module structural limits, mirroring the `wasmtime serve` config builder (`crates/cli-flags/src/lib.rs` at the `release-46.0.0` tag):

- Keep `total_*` independent (do not pin `total_memories`/`total_tables` to the component-instance count). Provide sane independent defaults with headroom.
- Add per-component limits: `max_core_instances_per_component`, `max_memories_per_component`, `max_tables_per_component`.
- Add per-module limits: `max_memories_per_module`, `max_tables_per_module`.
- Add `max_core_instance_size` / `max_component_instance_size` if guests need more metadata than the defaults.
- Add `decommit_batch_size` (batch decommits to amortise syscalls).
- On Linux, expose `pagemap_scan` (uses the `PAGEMAP_SCAN` ioctl for faster linear-memory reset where the kernel supports it).

All exposed as `POOL_*` env vars with documented defaults, validated at start-up.

### Step 2 — Optional MPK + observability + benchmarking harness

- **Memory protection keys (MPK):** behind a feature flag / env toggle (`memory_protection_keys`, `max_memory_protection_keys`). MPK lets the pool pack more linear memories into less virtual address space, raising achievable density. Requires the `memory-protection-keys` Wasmtime feature and is Linux/x86_64-only; gate accordingly and fall back cleanly.
- **Observability:** emit metrics around instantiation (instantiation latency histogram, pool slot occupancy / exhaustion counter, warm-slot hit rate) so pool sizing can be tuned from real data rather than guesswork. Tie into the existing OpenTelemetry setup in `crates/otel`.
- **Benchmark harness:** a small load-test (e.g. against the `examples/http` guest) capturing p50/p99 instantiation + end-to-end latency and RSS, runnable via `cargo make`, to gate residency/MPK tuning and catch regressions.

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

1. Plan A, Step 1 (hoist binding lookups) — safe, immediate, no new config.
2. Plan A, Step 2 (residency + warm slots) — biggest latency win; benchmark.
3. Plan B, Step 1 (correct/parameterise pooling) — fixes the latent totals bug.
4. Plan B, Step 2 (MPK + observability + harness) — only after measuring.

## Open questions

- Target deployment density: how many concurrent guest instances per host do we size the pool for?
- Are guests trusted or untrusted? (Drives `async_stack_zeroing` and how aggressively we reuse warm slots.)
- Which platforms must we support? (MPK and `pagemap_scan` are Linux-specific.)
- Do any guests use multiple memories/tables or component-model GC? (Drives the per-component limits and GC tuning.)

