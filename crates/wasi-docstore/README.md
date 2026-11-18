# Omnia WASI `DocStore`

This crate provides the JSON document store interface for the Omnia runtime.

## Interface

Implements `omnia:docstore`, the interface the SDK's `DocumentDb` calls, over
one `DocStoreClient` per backend.

- A guest opens a collection by name and calls it through the handle `open`
  returns. A name the deployment does not declare fails with `not-found`.
  An absent document is `none`, not an error.
- A document is a JSON object stored under a string id. A body that is not
  a JSON object is rejected with `invalid` before the backend sees it.
- A write returns the document's revision, which a read of the same body
  reports. A `put` with `absent` creates the document only when the id is
  free, and otherwise returns a `conflict` carrying the document it found.
- `get-many`, `put-many`, and `delete-many` make one call per id. A
  `put-many` that gives an id twice writes nothing.
- A query's filter is a list of groups: every group must hold, and a group
  holds when any of its conditions does. A page holds at most 1,000
  documents, and its `next` cursor resumes the query.
- The host answers `unsupported` for what `DocStoreClient` cannot express
  yet:
  - a `put` with `matches`;
  - a `delete` with a revision;
  - an `array-contains` test.

See the [`DocStore` Interface Reference](../../docs/reference/docstore.md) for the operations, the portable query, the filter language and its host-enforced limits, and the backends.

## Backend

- **Default**: In-memory document store. Filters, sorting, and pagination are evaluated directly
  over the stored JSON; state is process-local and lost on exit. It serves the collections
  `DOCSTORE_COLLECTIONS` declares (comma-separated, default `default`).

- **Production**: [`omnia-azure-table`](https://github.com/augentic/omnia-backends/tree/main/crates/azure-table) (Azure Table Storage) — a one-line swap in the host, guests untouched (see the [Production Backends guide](https://github.com/augentic/omnia/blob/main/docs/guides/production-backends.md)).

## Usage

Add this crate to your `Cargo.toml` and use it in your runtime configuration:

```rust,ignore
use omnia_wasi_docstore::{DocStoreDefault, WasiDocStore};

omnia::runtime!({
    hosts: {
        WasiDocStore: DocStoreDefault,
    }
});
```

## License

MIT OR Apache-2.0
