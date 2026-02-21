# Omnia WASI Blobstore

This crate provides the Blobstore interface for the Omnia runtime.

## Interface

Implements the `wasi:blobstore` WIT interface.

## Backend

- **Default**: In-memory implementation. Data is not persisted across restarts.

## Usage

Add this crate to your `Cargo.toml` and use it in your runtime configuration:

```rust,ignore
use omnia::runtime;
use omnia_wasi_blobstore::BlobstoreDefault;

omnia::runtime!({
    "blobstore": BlobstoreDefault,
});
```

## License

MIT OR Apache-2.0
