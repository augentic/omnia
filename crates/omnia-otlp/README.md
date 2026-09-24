# omnia-otlp

OTLP span and metric exporters for the [omnia](https://github.com/augentic/omnia)
runtime's host telemetry: [`Exporters`] attaches a gRPC span exporter and a
periodic metric exporter to omnia's console `SubscriberBuilder`, publishes the
OpenTelemetry providers process-wide once the subscriber is installed, and
exposes `flush` (batched telemetry survives a fast exit) and `resource` (the
process's OpenTelemetry resource, for hosts that report it to guests).

Depend on the `omnia` facade, not on this crate: the whole surface re-exports
there behind the `otlp` feature (`omnia::otlp::Exporters`, `omnia::otlp::flush`,
`omnia::otlp::resource`). A direct dependency on `omnia-otlp` (or on
`omnia-core`) is never needed by a deployment.

[`Exporters`]: https://docs.rs/omnia/latest/omnia/otlp/struct.Exporters.html

## License

MIT OR Apache-2.0
