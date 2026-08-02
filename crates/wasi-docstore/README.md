# Omnia WASI `DocStore`

This crate provides the JSON document store interface for the Omnia runtime.

## Interface

Implements the `wasi:docstore` WIT interface. Documents are stored as JSON bytes with a string primary key. Queries support filtering via a host-managed filter resource, sorting, pagination, and continuation tokens.

See the [`DocStore` Interface Reference](../../docs/reference/docstore.md) for the full WIT definition, SDK types, backend translator details, and host-enforced limits.

## Backend

- **Default**: In-memory document store. Filters, sorting, and pagination are evaluated directly over the stored JSON; state is process-local and lost on exit, like the other in-memory defaults.

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
