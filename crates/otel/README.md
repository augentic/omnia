# Omnia OpenTelemetry

OpenTelemetry tracing and metrics integration for the Omnia runtime. This crate initializes `tracing-subscriber`, OTLP span exporters, and metric readers with retry logic so that host runtimes report telemetry out-of-the-box.

> **Note:** This is a host-side library (not compiled for `wasm32`). It is called internally by `omnia::create()` during runtime startup. Most users do not need to depend on this crate directly.

## Usage

```rust,ignore
use omnia_otel::Telemetry;

// Minimal -- uses RUST_LOG for filtering, no OTLP export
Telemetry::new("my-service").build()?;

// With OTLP export to a collector
Telemetry::new("my-service")
    .endpoint("http://localhost:4317")
    .build()?;
```

The `OTEL_GRPC_URL` environment variable is also respected if no explicit endpoint is set.

## License

MIT OR Apache-2.0
