# Omnia WASI Messaging

This crate provides the Messaging interface for the Omnia runtime.

## Interface

Implements the `wasi:messaging` WIT interface.

## Backend

- **Default**: In-memory broadcast channel using `tokio::sync::broadcast`. Messages are only delivered to subscribers within the same process.

- **Production**: [`omnia-kafka`](https://github.com/augentic/omnia-backends/tree/main/crates/kafka) (Apache Kafka), [`omnia-nats`](https://github.com/augentic/omnia-backends/tree/main/crates/nats) (NATS) — a one-line swap in the host, guests untouched (see the [Production Backends guide](https://github.com/augentic/omnia/blob/main/docs/guides/production-backends.md)).

## Usage

Add this crate to your `Cargo.toml` and use it in your runtime configuration:

```rust,ignore
use omnia_wasi_messaging::{MessagingDefault, WasiMessaging};

omnia::runtime!({
    hosts: {
        WasiMessaging: MessagingDefault,
    }
});
```

## License

MIT OR Apache-2.0
