# OpenTelemetry Example

Demonstrates OpenTelemetry instrumentation for WebAssembly guests using `wasi-otel`.

## Quick Start

The quick start uses the default implementation of the wasi-otel host backend. This is a no-op
implementation for development use only as it logs telemetry data but doesn't export it anywhere.

```bash
# build the guest
cargo build --example otel-wasm --target wasm32-wasip2

# run the host
export RUST_LOG="info,wasi_otel=debug,omnia_wasi_http=debug,otel=debug"
cargo run --example otel -- run ./target/wasm32-wasip2/debug/examples/otel_wasm.wasm
```

## Test

```bash
curl --header 'Content-Type: application/json' -d '{"text":"hello"}' http://localhost:8080
```

## Cleanup

```bash
docker compose -f docker/otelcol.yaml down -v
```

## Using an OpenTelemetry Collector Backend

To use an OpenTelemetry Collector backend, you need to set the `OTEL_GRPC_URL` environment variable to the
address of the OpenTelemetry Collector.

## Prerequisites

Start an OpenTelemetry Collector instance:

```bash
docker compose -f docker/otelcol.yaml up -d
```

Modify the runtime.rs file to use the OpenTelemetry Collector backend:

```rust,ignore
...
use omnia::opentelemetry::WasiOtelCtx;

omnia::runtime!({
    main: true,
    hosts: {
        WasiHttp: HttpDefault,
        WasiOtel: WasiOtelCtx,
    }
});
...
```

```bash
export OTEL_GRPC_URL="http://localhost:4317"
export RUST_LOG="info,wasi_otel=debug,omnia_wasi_http=debug,otel=debug"
cargo run --example otel -- run ./target/wasm32-wasip2/debug/examples/otel_wasm.wasm
```
