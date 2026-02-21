# WASI OpenTelemetry Host

This crate provides a WASI-enabled OpenTelemetry host for use in Omnia WebAssembly components.

## Transport

This implementation uses gRPC to export telemetry data to an OpenTelemetry collector. The gRPC endpoint can be configured using the `OTEL_GRPC_URL` environment variable (default: `http://localhost:4317`).

## Example Setup

See the compose.yaml.example file in this crate's directory for a Docker Compose file that includes images for the service as well as an OTel collector, Prometheus for metrics and monitoring and Jaeger for tracing visualization.
