# Performance Tuning

Omnia instantiates a fresh guest instance for every request. This page explains where the cost of that model lives, which knobs move it, and how to measure before and after. The full variable list is in [Configuration](../reference/configuration.md); this guide is about which ones to reach for.

## The execution model, briefly

Guests are compiled and pre-instantiated **once** at startup. Per request, the runtime allocates a store, instantiates the guest from its `InstancePre`, runs the handler, and tears everything down. The recurring costs are therefore instance allocation (linear memory, tables, async stacks) and memory zeroing — not compilation. The pooling allocator (`POOLING=true`, the default) exists to recycle those allocations.

## Measure first

The [`bench`](../../examples/bench/) harness drives concurrent keep-alive HTTP load and reports throughput plus p50/p90/p99/p99.9 latency, with no external tools:

```bash
cargo build --example http-wasm --target wasm32-wasip2
RUST_LOG=warn cargo run --example http -- run ./target/wasm32-wasip2/debug/examples/http_wasm.wasm
# second terminal:
BENCH_CONCURRENCY=64 BENCH_DURATION_SECS=30 cargo run --example http-bench
```

Watch two signals while it runs:

- **Pool occupancy gauges** — with `RUST_LOG=info`, the host logs pool-occupancy metrics every `POOL_METRICS_INTERVAL_MS` (default 5s; set `1000` while tuning). If occupancy hits the pool ceilings, requests queue.
- **Resident memory** — `ps -o rss= -p $(pgrep -f 'my-runtime run')` while under load, since most pooling knobs trade memory for latency.

## Knobs, in the order to try them

### 1. Memory residency on slot reuse

By default a pooled slot's linear memory is fully decommitted between uses, so every request pays page-fault and zeroing costs. Keeping the first N bytes resident is usually the single biggest win for small guests:

```bash
POOL_MEMORY_KEEP_RESIDENT=1048576   # keep 1 MiB resident per pooled memory
```

`POOL_TABLE_KEEP_RESIDENT` and `POOL_ASYNC_STACK_KEEP_RESIDENT` are the same trade for tables and async stacks. Cost: resident memory scales with `POOL_MAX_UNUSED_WARM_SLOTS` × kept bytes.

### 2. Pool sizing

The defaults (1000 instances/memories/tables/stacks, 100 warm slots) suit moderate concurrency. Raise `POOL_MAX_INSTANCES` and the `POOL_TOTAL_*` ceilings if occupancy gauges show saturation under your target load; raise `POOL_MAX_UNUSED_WARM_SLOTS` if your steady-state concurrency exceeds 100 and you can spend the memory.

### 3. Batched decommit

`POOL_DECOMMIT_BATCH_SIZE` batches slot decommits to amortise syscalls — worth testing at high request rates.

### 4. Linux-specific options

- `POOL_PAGEMAP_SCAN=auto` — uses the `PAGEMAP_SCAN` ioctl for cheaper memory reset on kernels that support it.
- `POOL_MEMORY_PROTECTION_KEYS=auto` (requires the `mpk` cargo feature, x86-64) — packs pooled memories with MPK, trading virtual address space for density.

### 5. Startup latency

Per-request tuning doesn't help cold starts. For those, pre-compile guests (`compile` → `.bin`) so startup skips Cranelift entirely — see [Deploying Omnia](deployment.md#ahead-of-time-compilation).

## Limits that interact with latency

- `GUEST_TIMEOUT_MS` (default 30s) bounds each invocation wall-clock; `EPOCH_TICK_MS` (default 10ms) is the granularity at which CPU-bound guests yield and timeouts are enforced. Lowering the tick tightens timeout precision at slight overhead.
- `MAX_FUEL` adds per-instruction metering. Leave it at `0` (off) unless you need deterministic CPU budgets — metering has measurable overhead and is compile-affecting.
- `MAX_MEMORY_BYTES` caps guest memory growth; `POOL_MAX_MEMORY_BYTES` must be at least the largest guest's requirement or instantiation falls back off the pool.

## What not to tune

Guest lookup, routing, and dispatch are in-process and effectively free relative to instantiation. If p99 is high, look at the pool gauges and the guest's own work (outbound calls, backend latency — visible in OTel traces via `OTEL_GRPC_URL`) before touching anything else.
