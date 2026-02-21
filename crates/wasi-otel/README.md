# Omnia WASI OpenTelemetry

This crate provides the OpenTelemetry interface for the Omnia runtime.

## Interface

Implements the `wasi:otel` WIT interface.

## Backend

Uses `opentelemetry` and `tracing` crates to export telemetry data.

## Configuration

- **`OTEL_GRPC_URL`**: The gRPC endpoint for the OpenTelemetry collector (default: `http://localhost:4317`).

## Usage

Add this crate to your `Cargo.toml` and use it in your runtime configuration:

```rust,ignore
use omnia::runtime;
use omnia_wasi_otel::DefaultOtel;

omnia::runtime!({
    "otel": DefaultOtel,
});
```

## License

MIT OR Apache-2.0
