# Omnia WASI `JsonDb`

This crate provides the JSON document store interface for the Omnia runtime.

## Interface

Implements the `wasi:jsondb` WIT interface. Documents are stored as JSON bytes with a string primary key. Queries support filtering via a host-managed filter resource, sorting, pagination, and continuation tokens.

## Backend

- **Default**: Uses `PoloDB` (MongoDB-compatible embedded database). Configure the database path with the `JSONDB_DATABASE` environment variable (default: `omnia-jsondb.polodb` in the system temp directory).

## Usage

Add this crate to your `Cargo.toml` and use it in your runtime configuration:

```rust,ignore
use omnia::runtime;
use omnia_wasi_jsondb::JsonDbDefault;

omnia::runtime!({
    "jsondb": JsonDbDefault,
});
```

## License

MIT OR Apache-2.0
