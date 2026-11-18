# Omnia WASI OpenTelemetry

This crate provides the OpenTelemetry interface for the Omnia runtime.

## Interface

Implements `omnia:otel`, the interface the guest telemetry runtime below
exports through, over one `OtelClient` per backend.

- Records carry the names of OTLP, the OpenTelemetry Protocol, and mean
  what OTLP means by them. Every time is in nanoseconds since the Unix
  epoch.
- Every backend serves `traces` and `metrics`. `logs` is optional, and this
  host answers every `logs` export with `unsupported`.
- The guest exports its own telemetry. The host adds the deployment's
  resource and hands each call to the backend as one OTLP request.
- `current-span-context` is the host's span for the call the guest is
  serving. A root span that takes it as its parent joins the host's trace.
- Every backend carries portable telemetry, and may reject any other with
  `invalid`:
  - a trace id is 32 lowercase hexadecimal digits and a span id is 16,
    neither all zeros;
  - an attribute key is not empty, and a list of attributes holds each key
    once;
  - the telemetry of one call is at most 4 mebibytes as an OTLP request.
- The host itself refuses, with `invalid`, an id that is not portable and a
  `json-value` that is not JSON. It then exports none of the call's
  telemetry.
- `ok` means the host took the telemetry for delivery. An empty export
  reaches no backend, and a host with no telemetry installed drops what it
  is given.
- A backend picks a failure's case by returning an `omnia_wasi_otel::Error`,
  and any other failure is `other`.

## Backend

- **Default**: `OtelDefault` logs how many spans and metrics each export
  carries, at `debug`, and sends them nowhere. It is for development only.

## Configuration

- **`OTEL_EXPORTER_OTLP_ENDPOINT`**: The gRPC endpoint for the OpenTelemetry collector, for the host's own spans and — through [`omnia-opentelemetry`](https://github.com/augentic/omnia-backends/tree/main/crates/opentelemetry) — the guest telemetry it receives. Unset, the host attaches no exporter, and `OtelDefault` (below) drops guest telemetry whatever is set.

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

A guest's root spans join the host's trace: the host span live when the guest starts a root span becomes its parent, and without one the guest's spans form a trace of their own. The runtime opens one at `INFO` around every `wasi:cli/run` drive (`cli-run`) and every trigger request (`http-request`, `messaging-handle`, `websocket-handle`), so a command run's guest spans join its trace at its default level and a server's from `-v` (`RUST_LOG=info`); its default `warn` disables the span. The console's refinement of the runtime's own crates ([Verbosity flags](https://github.com/augentic/omnia/blob/main/docs/reference/configuration.md#verbosity-flags)) is the console layer's alone and never disables a span.

### Guest

On `wasm32` the same crate is the guest's telemetry runtime. `#[instrument]` on an `async fn` opens a span, and the outermost one runs the lifecycle (`scope`): it installs a `tracing` subscriber on first use and exports every buffered span and recorded metric to the host as the function returns. `command!` does the same for a CLI entry. `omnia_wasi_otel::omnia` holds the raw bindings, for a guest that builds its own telemetry.

```rust,ignore
#[omnia_wasi_otel::instrument(name = "http_guest_handle")]
async fn handle(request: Request) -> Response {
    tracing::info!("handling");
    // ...
}
```

Console output (events only, to stderr) follows the `RUST_LOG` the guest's WASI environment carries (the runtime's tracing filter: a `-v`/`-q` flag's level, else the process `RUST_LOG`'s, else the mode's default, with the process `RUST_LOG`'s targeted directives on top), defaulting to `error` when there is none, and colours its lines unless that environment carries `NO_COLOR` — which the runtime sets when the console it shares is not a terminal; `flush` exports on demand.

## License

MIT OR Apache-2.0
