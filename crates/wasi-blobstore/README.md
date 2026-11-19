# Omnia WASI Blobstore

This crate provides the Blobstore interface for the Omnia runtime.

## Interface

Implements `omnia:blobstore`, the interface the SDK's `blobstore` module binds, over
one `BlobStoreClient` per backend.

- A guest opens a container by name and calls it through the handle `open`
  returns. A container that does not exist fails `open` with `not-found`.
  An absent object is `none`, not an error.
- Every backend stores the portable container and object names the SDK's
  `blobstore` module defines. A backend may reject any other name with `invalid`.
- `get` streams an object, or the `byte-range` of it a guest selects,
  clamped to the object's length.
- `put` streams an object in, and stores it only once the writer resolves
  `finished` with `ok`.
- `list` answers one page of the names under a prefix, flat or folded at
  the next `/`, with the cursor that resumes it.
- `admin`, `uploads`, and `presign` are optional. `uploads` and `presign`
  are unstable, and a guest binds one by enabling the feature of its name.
- The host answers `unsupported` for what `Container` cannot express yet:
  - a `put`, `delete`, `copy`, or `rename` with a precondition;
  - a `put` with attributes;
  - `list-containers`;
  - every `uploads` and `presign` call.
- Until a backend reports one, an `etag` derives from the object's write
  time and size, so two writes of one size within a second share it.

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
