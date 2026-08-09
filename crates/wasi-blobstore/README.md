# Omnia WASI Blobstore

This crate provides the Blobstore interface for the Omnia runtime.

## Interface

Implements the `wasi:blobstore` WIT interface.

## Backend

- **Default**: In-memory implementation. Data is not persisted across restarts.

- **Production**: [`omnia-azure-blob`](https://github.com/augentic/omnia-backends/tree/main/crates/azure-blob) (Azure Blob Storage), [`omnia-filesystem`](https://github.com/augentic/omnia-backends/tree/main/crates/filesystem) (local filesystem, durable and network-free), [`omnia-mongodb`](https://github.com/augentic/omnia-backends/tree/main/crates/mongodb) (MongoDB), [`omnia-nats`](https://github.com/augentic/omnia-backends/tree/main/crates/nats) (NATS JetStream object store) — a one-line swap in the host, guests untouched (see the [Production Backends guide](https://github.com/augentic/omnia/blob/main/docs/guides/production-backends.md)).

## Usage

Add this crate to your `Cargo.toml` and use it in your runtime configuration:

```rust,ignore
use omnia_wasi_blobstore::{BlobstoreDefault, WasiBlobstore};

omnia::runtime!({
    hosts: {
        WasiBlobstore: BlobstoreDefault,
    }
});
```

## License

MIT OR Apache-2.0
