# Omnia Native

In-process bridge from [`omnia-sdk`](../omnia-sdk) capability traits to [`omnia-backend`](../omnia-backend) service contracts, plus native HTTP and messaging server drivers.

## Overview

- **`Direct<T>`** wraps an already-connected service client (or default implementation) and implements the corresponding `omnia-sdk` capability trait in-process (`Direct::new(client)` or `Direct::with_mounts(client, mounts)`), sharing the Omnia WASI host's bucket and locker names, connection-per-call lifetime, `ToolSession` budgets, `cap-std` `MountRegistry` confinement, and value types.
- **`Serve`** drives one or more `axum::Router` HTTP listeners and `omnia_sdk::api::messaging::Router` consumers concurrently, with graceful shutdown via `Serve::shutdown` (defaulting to `SIGINT` + `SIGTERM`) and OpenTelemetry flushing (behind the default `telemetry` feature).
- **`RouterHandler<P>`** adapts an `omnia_sdk::api::messaging::Router<P>` to `omnia_backend::messaging::Handler`.

## License

MIT OR Apache-2.0
