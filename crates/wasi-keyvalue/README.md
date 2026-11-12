# Omnia WASI Key-Value

This crate provides the Key-Value interface for the Omnia runtime.

## Interface

Implements `omnia:keyvalue`, the interface the SDK's `StateStore` calls, and
the `wasi:keyvalue` draft for raw-binding guests. Both are served from one
`Bucket` per backend.

- A guest opens a bucket by name. A name the deployment does not declare
  fails with `not-found`.
- Every backend stores a portable key: one that is not empty, holds only
  ASCII letters, digits, `.`, `_`, and `-`, and neither starts nor ends with
  `.`. A backend may reject any other key with `invalid`.
- The backend enforces a TTL. An entry written without one never expires, and
  a zero TTL is rejected before the backend sees it.
- `compare-and-swap` and `increment` apply a TTL only when they create the
  entry. An existing entry keeps the time it has left.

## Backend

- **Default**: In-memory cache using `moka`. Data is not persisted across restarts. It serves the
  buckets `KEYVALUE_BUCKETS` declares (comma-separated, default `default`).

- **Production**: [`omnia-redis`](https://github.com/augentic/omnia-backends/tree/main/crates/redis) (Redis), [`omnia-nats`](https://github.com/augentic/omnia-backends/tree/main/crates/nats) (NATS JetStream) — a one-line swap in the host, guests untouched (see the [Production Backends guide](https://github.com/augentic/omnia/blob/main/docs/guides/production-backends.md)).

## Usage

Add this crate to your `Cargo.toml` and use it in your runtime configuration:

```rust,ignore
use omnia_wasi_keyvalue::{KeyValueDefault, WasiKeyValue};

omnia::runtime!({
    hosts: {
        WasiKeyValue: KeyValueDefault,
    }
});
```

## License

MIT OR Apache-2.0
