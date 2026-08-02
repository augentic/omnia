# Omnia WASI OpenTelemetry

This crate provides the OpenTelemetry interface for the Omnia runtime.

## Interface

Implements the `wasi:otel` WIT interface.

## Backend

Uses `opentelemetry` and `tracing` crates to export telemetry data.

## Configuration

- **`OTEL_GRPC_URL`**: The gRPC endpoint for the OpenTelemetry collector (default: `http://localhost:4317`).

- **Production**: [`omnia-opentelemetry`](https://github.com/augentic/omnia-backends/tree/main/crates/opentelemetry) (OTLP gRPC collector export) — a one-line swap in the host, guests untouched (see the [Production Backends guide](https://github.com/augentic/omnia/blob/main/docs/guides/production-backends.md)).

## Usage

Add this crate to your `Cargo.toml` and use it in your runtime configuration:

```rust,ignore
use omnia_wasi_otel::{OtelDefault, WasiOtel};

omnia::runtime!({
    hosts: {
        WasiOtel: OtelDefault,
    }
});
```

## License

MIT OR Apache-2.0
