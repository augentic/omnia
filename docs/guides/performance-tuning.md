# Performance Tuning

Omnia instantiates a fresh guest instance for every request. This page explains where the cost of that model lives, which knobs move it, and how to measure before and after. The full variable list is in [Configuration](../reference/configuration.md); this guide is about which ones to reach for.

## The execution model, briefly

Guests are compiled and pre-instantiated **once** at startup. Per request, the runtime allocates a store, instantiates the guest from its `InstancePre`, runs the handler, and tears everything down. The recurring costs are therefore instance allocation (linear memory, tables, async stacks) and memory zeroing — not compilation. The pooling allocator (`POOLING=true`, the default) exists to recycle those allocations.

## Measure first

Drive concurrent keep-alive HTTP load at a running host with any load-testing tool (for example [`wrk`](https://github.com/wg/wrk) or [`hey`](https://github.com/rakyll/hey)) and record throughput plus tail latency (p99 and up) before and after each change. The [`http`](../../examples/http/) example makes a convenient target:

```bash
cargo build --example http-wasm --target wasm32-wasip3
cargo run --example http -- run ./target/wasm32-wasip3/debug/examples/http_wasm.wasm
# second terminal:
hey -z 30s -c 64 http://127.0.0.1:8080/
```

Watch two signals while it runs:

- **Occupancy gauges** — the host samples the pool onto its `pool_*` gauges, and the concurrency gate onto its `invocations_*` gauges, every `POOL_METRICS_INTERVAL_MS` (default 5s; set `1000` while tuning). The gauges reach the collector `OTEL_EXPORTER_OTLP_ENDPOINT` names whatever `RUST_LOG` is. `RUST_LOG=omnia=trace` also prints each sample on the console, and the console line adds `pool_unused_warm_memories` and `pool_unused_memory_bytes_resident`, which the collector does not receive. When the gate is full, HTTP answers `503` and deliveries wait; [Pool sizing](#2-pool-sizing) reads the gauges.
- **Resident memory** — `ps -o rss= -p $(pgrep -f 'my-runtime run')` while under load, since most pooling knobs trade memory for latency.

## Knobs, in the order to try them

### 1. Memory residency on slot reuse

By default a pooled slot's linear memory is fully decommitted between uses, so every request pays page-fault and zeroing costs. Keeping the first N bytes resident is usually the single biggest win for small guests:

```bash
POOL_MEMORY_KEEP_RESIDENT=1048576   # keep 1 MiB resident per pooled memory
```

`POOL_TABLE_KEEP_RESIDENT` and `POOL_ASYNC_STACK_KEEP_RESIDENT` are the same trade for tables and async stacks. Cost: resident memory scales with `POOL_MAX_UNUSED_WARM_SLOTS` × kept bytes.

That cost is paid while the host is idle too. `POOL_IDLE_RELEASE_MS` gives the kept linear memory back to the OS once no guest has been invoked or live for that long, measured from the last guest invocation and acted on within about a second, so a bursty service keeps warm slots through a burst and sheds them between bursts; the first requests after a quiet spell pay the cold-slot cost again:

```bash
POOL_IDLE_RELEASE_MS=60000          # release warm memory after a minute without a guest invocation
```

### 2. Pool sizing

The defaults (1000 instances, 100 warm slots) suit moderate concurrency. The `POOL_TOTAL_*` ceilings follow `POOL_MAX_INSTANCES` unless set ([configuration reference](../reference/configuration.md#instance-pooling)), so it is the one knob to raise when occupancy gauges show saturation under your target load. Raise `POOL_MAX_UNUSED_WARM_SLOTS` if your steady-state concurrency exceeds 100 and you can spend the memory.

The [concurrency gate](../reference/configuration.md#concurrency-gate) admits `MAX_CONCURRENT_INVOCATIONS` invocations at once, and each holds an instance. A call from one guest to another takes a further instance, outside the gate. Size the pool for both:

```text
POOL_MAX_INSTANCES >= MAX_CONCURRENT_INVOCATIONS x (1 + hops in flight per invocation)
```

The hops in flight are the calls between guests one invocation holds open at once: one for a guest that calls one other guest, two for A calling B calling C. Unset, the gate is as wide as the pool, which suits a deployment whose guests make no such calls. At a raised `POOL_DECOMMIT_BATCH_SIZE`, that holds only while the pool's totals include the [decommit margin](#3-batched-decommit), as the unset ones do. A call that finds the pool full fails at once, with `the instance pool is full`. An HTTP request the gate admitted but the pool has no room for is answered `503`, and the host logs the same message at `warn`.

The gate's gauges, and its one counter, read as follows:

- `invocations_in_flight` is how many slots are held. Pinned at the gate's size, the gate is what limits throughput.
- `invocations_waiting` is how many message deliveries, command runs, and calls through `Runtime::dispatcher` wait for a slot. Waiters take freed slots in the order they arrived, ahead of any request, so HTTP and WebSocket are refused while it stays above zero. No more than `MESSAGING_MAX_IN_FLIGHT` deliveries hold or wait for a slot. One past them waits off the gate, where this gauge does not count it.
- `invocations_refused` counts the HTTP requests and WebSocket events refused at a full gate, and `processing_errors` does not count them. HTTP answers each with `503`. A request the gate admitted but the pool had no room for counts on `processing_errors` instead, though HTTP answers it `503` too.

A slot is held for as long as its invocation runs:

- A command's run holds one from start to exit, so a long-running command leaves one fewer for the triggers serving beside it.
- A guest that waits on another delivery in the same deployment, such as a reply its own messaging trigger handles, holds its slot while it waits. When such guests hold every slot, or messaging's whole share of `MESSAGING_MAX_IN_FLIGHT`, the deliveries they wait on queue until the guests time out.

The table and stack pools are mapped when the engine is built, four tables and two stacks for each instance. A host with strict overcommit is charged for the whole mapping, about 625 MiB of tables and 4 GiB of stacks at the defaults. A raised `POOL_DECOMMIT_BATCH_SIZE` maps the [decommit margin](#3-batched-decommit)'s tables too, and its stacks while `ASYNC_STACK_ZEROING` is on.

### 3. Batched decommit

`POOL_DECOMMIT_BATCH_SIZE` (default 1) is how many freed pool slots a thread queues before it resets them together. At 1, a slot is reset as its instance is dropped, before the invocation's slot of the [concurrency gate](../reference/configuration.md#concurrency-gate) is freed.

A batch saves syscalls only on Linux 6.14 or later, where a whole batch resets in one `process_madvise` call if seccomp allows it. wasmtime stops calling `process_madvise` for good once it is refused, as some container seccomp profiles refuse it, and then resets each slot on its own. Raise the batch only where a measurement shows a gain.

A raised batch holds freed slots back from reuse until their batch resets. The unset `POOL_TOTAL_*` add the [decommit margin](../reference/configuration.md#instance-pooling) to cover them: min(cores, 16) × (`POOL_DECOMMIT_BATCH_SIZE` − 1) more memories and tables, and as many stacks while `ASYNC_STACK_ZEROING` is on. Each extra memory reserves the address space of a pooled memory, which is `MEMORY_RESERVATION` (4 GiB on a 64-bit host) or `POOL_MAX_MEMORY_BYTES` if larger, plus `MEMORY_GUARD_SIZE` (32 MiB).

```bash
POOL_DECOMMIT_BATCH_SIZE=16   # with 16 or more cores, 240 more memories and tables
```

### 4. Linux-specific options

- `POOL_PAGEMAP_SCAN=auto` — uses the `PAGEMAP_SCAN` ioctl for cheaper memory reset on kernels that support it.

### 5. Streaming throughput

A guest moves a large body or file in bounded steps:

- `HTTP_BODY_BUFFER_CHUNKS` (default `1`) is how many body chunks, each up to 1 MiB, a guest may write ahead of the network. It applies to the responses a guest serves and the requests it sends. Raising it lets a guest run ahead of a slow peer, at up to that many chunks of memory per body in flight.
- `WASI_MAX_IO_SIZE` (default 64 KiB) is the largest single file or socket read or write the host performs for a guest. Raising it lets a guest copy a large file in fewer calls.

```bash
HTTP_BODY_BUFFER_CHUNKS=4   # a guest runs up to four chunks ahead of its peer
WASI_MAX_IO_SIZE=1048576    # file and socket I/O in 1 MiB steps
```

### 6. Startup latency

Per-request tuning doesn't help cold starts. For those, pre-compile guests (`compile` → `.bin`) so startup skips Cranelift entirely — see [Deploying Omnia](deployment.md#ahead-of-time-compilation).

## Limits that interact with latency

- `GUEST_TIMEOUT_MS` (default 30s) bounds each server invocation and each server-rooted link-dispatch hop wall-clock (a command-mode chain, including its link hops, is uncapped); `EPOCH_TICK_MS` (default 10ms) is the granularity at which CPU-bound guests yield and timeouts are enforced. Lowering the tick tightens timeout precision at slight overhead.
- `GUEST_TIMEOUT_MS` stops at an HTTP response's head. The body that streams after it keeps its instance and its slot of the gate until it ends, so a slow or endless body holds capacity. `HTTP_STREAM_TIMEOUT_MS` (unset by default) caps it, counted from the head, and cuts a body still streaming at the cap.
- `MAX_FUEL` adds per-instruction metering. Leave it at `0` (off) unless you need deterministic CPU budgets — metering has measurable overhead and is compile-affecting.
- `MAX_MEMORY_BYTES` caps guest memory growth; `POOL_MAX_MEMORY_BYTES` must be at least the largest guest's requirement or instantiation falls back off the pool.

## What not to tune

Guest lookup, routing, and dispatch are in-process and effectively free relative to instantiation. If p99 is high, look at the pool and gate gauges and the guest's own work (outbound calls, backend latency — visible in OTel traces via `OTEL_EXPORTER_OTLP_ENDPOINT`) before touching anything else.
