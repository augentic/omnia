# Omnia WASI OpenTelemetry

This crate provides the OpenTelemetry interface for the Omnia runtime.

## Interface

Implements the `wasi:otel` WIT interface.

## Backend

Uses `opentelemetry` and `tracing` crates to export telemetry data.

## Configuration

- **`OTEL_GRPC_URL`**: The gRPC endpoint for the OpenTelemetry collector. Unset defers to `OTEL_EXPORTER_OTLP_*`; with neither, the host attaches no exporter and guest telemetry it receives is dropped.

- **Production**: [`omnia-opentelemetry`](https://github.com/augentic/omnia-backends/tree/main/crates/opentelemetry) (OTLP gRPC collector export) — a one-line swap in the host, guests untouched (see the [Production Backends guide](https://github.com/augentic/omnia/blob/main/docs/guides/production-backends.md)).

## Usage

### Host

Add this crate to your `Cargo.toml` and use it in your runtime configuration:

```rust,ignore
use omnia_wasi_otel::{OtelDefault, WasiOtel};

omnia::runtime!({
    hosts: {
        WasiOtel: OtelDefault,
    }
});
```

Guest spans are grafted onto the host trace: the host span live when the guest exports becomes their parent, so drive a guest inside an enabled `tracing` span (the trigger hosts open one per request at `DEBUG`).

### Guest

On `wasm32` the same crate is the guest's telemetry runtime. `#[instrument]` on an `async fn` opens a span, and the outermost one runs the lifecycle (`scope`): it installs a `tracing` subscriber on first use and exports every buffered span and recorded metric to the host as the function returns. `command!` does the same for a CLI entry.

```rust,ignore
#[omnia_wasi_otel::instrument(name = "http_guest_handle")]
async fn handle(request: Request) -> Response {
    tracing::info!("handling");
    // ...
}
```

Console output (events only, to stderr) follows the `RUST_LOG` the guest's WASI environment carries (the runtime's tracing filter: a `-v`/`-q` flag's level, else the process `RUST_LOG`'s, else the mode's default, with the process `RUST_LOG`'s targeted directives on top), defaulting to `error` when there is none; `flush` exports on demand.

## License

MIT OR Apache-2.0
