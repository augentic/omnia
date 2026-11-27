# OpenTelemetry Example

Demonstrates OpenTelemetry instrumentation for WebAssembly guests using `wasi-otel`.

## Quick Start

The runtime wires the default `OtelDefault` backend, and the quick start sets no collector, so the guest's telemetry goes nowhere. Set `OTEL_EXPORTER_OTLP_ENDPOINT` to a collector's address and the guest's spans and metrics export with the host's own. The runtime needs no change.

```bash
make example otel
```

Or, more manually, for debugging:

```bash
# build the guest
cargo build --example otel-wasm --target wasm32-wasip3

# run the host
cargo run --example otel -- -v run ./target/wasm32-wasip3/debug/examples/otel_wasm.wasm
```

## Test

```bash
curl --header 'Content-Type: application/json' -d '{"text":"hello"}' http://localhost:8080
```

## Collector, Jaeger, and Prometheus

To see a guest's spans in Jaeger and its metrics in Prometheus, run the [`otel` example in `omnia-backends`](https://github.com/augentic/omnia-backends/tree/main/examples/otel). It brings its own collector stack.
