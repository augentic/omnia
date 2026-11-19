# Omnia WASI Messaging

This crate provides the Messaging interface for the Omnia runtime.

## Interface

Implements `omnia:messaging`, the interface the SDK's `Publisher` and
`messaging::export!` call, over one `Endpoint` per backend.

- A guest connects to a broker by name. The deployment declares one,
  `default`, and any other name fails with `not-found`.
- Every backend carries a portable topic, and may reject any other with
  `invalid`. A portable topic is one or more segments joined by `.`, and
  each segment is not empty and holds only ASCII letters, digits, `_`, and
  `-`.
- A message is a record of its payload, content type, ordering key, and
  the caller's own metadata. The content type and key reach the backend as
  the `content-type` and `key` metadata entries, and come back out of them
  on delivery.
- A subscribing guest exports `incoming-handler`. The host routes each
  delivery to a guest by topic, and sets its `deadline` to when it stops
  waiting.
- What `handle` returns is the backend's to act on. `ok` accepts the
  delivery, `reject` settles it for good, and `release` gives it back for
  another delivery after any delay it names. No backend reads the
  settlement yet.
- `request-reply` is optional, and awaits one reply. A zero timeout is
  `invalid`, and so is a reply to an empty address.
- `consumer`, `log`, and `admin` are unstable, and a guest binds one by
  enabling the feature of its name. The host answers every call of them
  with `unsupported`, since `Endpoint` cannot carry them yet.
- A delivery carries no `position`, `attempt`, or `published-at` yet.

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
