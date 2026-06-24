# HTTP Benchmark Harness

A self-contained load-test harness for the [`http`](../http) example. It drives concurrent keep-alive requests at a running host and reports throughput plus p50/p90/p99/p99.9 latency, so pooling-allocator tuning (residency, totals, MPK) can be measured and regressions caught. No external load tools are required.

## Quick Start

```bash
# build the guest, host, and harness
cargo build --example http-wasm --target wasm32-wasip2
cargo build --example http
cargo build --example http-bench

# run the host (listens on 127.0.0.1:8080)
export RUST_LOG=warn
cargo run --example http -- run ./target/wasm32-wasip2/debug/examples/http_wasm.wasm
```

In a second terminal, drive load against the running host:

```bash
cargo run --example http-bench
```

## Configuration

The harness reads these optional environment variables:

- `BENCH_ADDR` — host address to target (default `127.0.0.1:8080`)
- `BENCH_CONCURRENCY` — number of concurrent connections (default `32`)
- `BENCH_DURATION_SECS` — how long to sustain load (default `10`)
- `BENCH_TIMEOUT_SECS` — per-request timeout (default `5`)

```bash
BENCH_CONCURRENCY=64 BENCH_DURATION_SECS=30 cargo run --example http-bench
```

Tune the host under test with the `POOL_*` runtime options (see [`crates/omnia/src/options.rs`](../../crates/omnia/src/options.rs)) to compare pooling configurations. For example, keep linear memory resident on slot reuse and set `RUST_LOG=info` to emit the periodic pool-occupancy gauges:

```bash
POOL_MEMORY_KEEP_RESIDENT=1048576 POOL_METRICS_INTERVAL_MS=1000 RUST_LOG=info \
  cargo run --example http -- run ./target/wasm32-wasip2/debug/examples/http_wasm.wasm
```

## Measuring Memory

Sample the host's resident memory (RSS) while the benchmark runs:

```bash
ps -o rss= -p "$(pgrep -f 'examples/http run')" | awk '{print $1 / 1024 " MiB"}'
```
