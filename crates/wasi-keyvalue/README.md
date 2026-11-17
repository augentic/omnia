# Omnia WASI Key-Value

This crate provides the Key-Value interface for the Omnia runtime.

## Interface

Implements `omnia:keyvalue`, the interface the SDK's `StateStore` calls, over
one `Bucket` per backend.

- A guest opens a bucket by name and calls it through the handle `open`
  returns. A name the deployment does not declare fails with `not-found`.
  An absent key is `none`, not an error.
- Every backend stores the portable keys the SDK's `StateStore` defines. A
  backend may reject any other key with `invalid`.
- A write names its lifetime. `forever` and `ttl-ms` replace an entry's
  lifetime. `keep` and `keep-or-ttl-ms` leave an existing entry the time it
  has left. A zero TTL is rejected before the backend sees it.
- A write returns the entry's revision. A `set` with a precondition writes
  only while it holds, and otherwise returns a `conflict` carrying the entry
  it found.
- `increment` stores a counter as decimal text, which `get` reads back.
- `keys` is optional, and pages the keys under a prefix.
- The host answers `unsupported` for what `Bucket` cannot express yet:
  - a `delete` with a revision;
  - a `set` with `matches` and `forever` or `ttl-ms`;
  - an `increment` with `forever` or `ttl-ms`.

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
