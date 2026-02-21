# Omnia WASI Messaging

This crate provides the Messaging interface for the Omnia runtime.

## Interface

Implements the `wasi:messaging` WIT interface.

## Backend

- **Default**: In-memory broadcast channel using `tokio::sync::broadcast`. Messages are only delivered to subscribers within the same process.

## Usage

Add this crate to your `Cargo.toml` and use it in your runtime configuration:

```rust,ignore
use omnia::runtime;
use omnia_wasi_messaging::MessagingDefault;

omnia::runtime!({
    "messaging": MessagingDefault,
});
```

## License

MIT OR Apache-2.0
