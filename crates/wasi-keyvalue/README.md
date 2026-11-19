# Omnia WASI Key-Value

This crate provides the Key-Value interface for the Omnia runtime.

## Interface

Implements `omnia:keyvalue`, the interface the SDK's `keyvalue` module binds, over
one `Bucket` per backend.

- A guest opens a bucket by name and calls it through the handle `open`
  returns. A name the deployment does not declare fails with `not-found`.
  An absent key is `none`, not an error.
- Every backend stores a portable bucket name, key, and value, and may
  reject any other with `invalid`:
  - a bucket name is not empty and holds only ASCII letters, digits, `_`,
    and `-`;
  - a key is one or more segments joined by `.`, at most 255 bytes in all,
    and each segment is not empty and holds only ASCII letters, digits,
    `_`, and `-`;
  - a value is at most 512 kibibytes.
- A write names its lifetime. `forever` and `ttl-ms` replace an entry's
  lifetime. `keep` and `keep-or-ttl-ms` leave an existing entry the time it
  has left. A zero TTL is rejected before the backend sees it.
- A write returns the entry's revision. A `set` with a precondition, or a
  `delete` with a revision, acts only while it holds, and otherwise returns
  a `conflict` carrying the entry it found.
- `increment` stores a counter as decimal text, which `get` reads back. Its
  `ttl-ms` applies only to a counter it creates.
- `keys` is optional, and pages the keys under a prefix.
- A `Bucket` checks a precondition in the same atomic step as its write. A
  backend that keeps no revision of its own reports `Revision::of` the
  stored bytes.

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
