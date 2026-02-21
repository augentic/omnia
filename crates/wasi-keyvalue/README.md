# Omnia WASI Key-Value

This crate provides the Key-Value interface for the Omnia runtime.

## Interface

Implements the `wasi:keyvalue` WIT interface.

## Backend

- **Default**: In-memory cache using `moka`. Data is not persisted across restarts.

## Usage

Add this crate to your `Cargo.toml` and use it in your runtime configuration:

```rust,ignore
use omnia::runtime;
use omnia_wasi_keyvalue::KeyValueDefault;

omnia::runtime!({
    "keyvalue": KeyValueDefault,
});
```

## License

MIT OR Apache-2.0
