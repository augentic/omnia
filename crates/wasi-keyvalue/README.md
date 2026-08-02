# Omnia WASI Key-Value

This crate provides the Key-Value interface for the Omnia runtime.

## Interface

Implements the `wasi:keyvalue` WIT interface.

## Backend

- **Default**: In-memory cache using `moka`. Data is not persisted across restarts.

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
