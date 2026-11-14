# Omnia WASI Blobstore

This crate provides the Blobstore interface for the Omnia runtime.

## Interface

Implements `omnia:blobstore`, the interface the SDK's `BlobStore` calls, and
the `wasi:blobstore` draft for raw-binding guests. Both are served from one
`BlobStoreClient` per backend.

- A call on an object in a container that does not exist fails with
  `not-found`. An absent object is `none`, not an error.
- Every backend stores the portable container and object names the SDK's
  `BlobStore` defines. A backend may reject any other name with `invalid`.
- `get-range` reads a half-open range, clamped to the object's length.
- `list` answers one page of the objects under a prefix, with the cursor
  that resumes it.

## Backend

- **Default**: In-memory implementation. Data is not persisted across restarts.

- **Production**: [`omnia-azure-blob`](https://github.com/augentic/omnia-backends/tree/main/crates/azure-blob) (Azure Blob Storage), [`omnia-filesystem`](https://github.com/augentic/omnia-backends/tree/main/crates/filesystem) (local filesystem, durable and network-free), [`omnia-mongodb`](https://github.com/augentic/omnia-backends/tree/main/crates/mongodb) (MongoDB), [`omnia-nats`](https://github.com/augentic/omnia-backends/tree/main/crates/nats) (NATS JetStream object store) — a one-line swap in the host, guests untouched (see the [Production Backends guide](https://github.com/augentic/omnia/blob/main/docs/guides/production-backends.md)).

## Usage

Add this crate to your `Cargo.toml` and use it in your runtime configuration:

```rust,ignore
use omnia_wasi_blobstore::{BlobStoreDefault, WasiBlobStore};

omnia::runtime!({
    hosts: {
        WasiBlobStore: BlobStoreDefault,
    }
});
```

## License

MIT OR Apache-2.0
