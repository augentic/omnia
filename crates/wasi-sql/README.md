# Omnia WASI SQL

This crate provides the SQL database interface for the Omnia runtime.

## Interface

Implements the `wasi:sql` WIT interface.

## Backend

- **Host**: Uses `rusqlite` to provide a `SQLite` backend. Supports both in-memory (`:memory:`) and file-based databases.

## Cargo features

- `sqlite` *(default)*: the bundled-SQLite default backend (`SqlDefault`). Disable it (`default-features = false`) when supplying your own `WasiSqlCtx` backend to skip compiling bundled `SQLite`.

## Features

### Guest ORM Layer

The guest module provides query builders for type-safe database operations:

- **Entity macro**: Declare database models with automatic trait implementations.
- **Query builders**: Fluent APIs for SELECT, INSERT, UPDATE, DELETE.
- **Joins & Filters**: Type-safe query construction.

- **Production**: [`omnia-postgres`](https://github.com/augentic/omnia-backends/tree/main/crates/postgres) (`PostgreSQL`) — a one-line swap in the host, guests untouched (see the [Production Backends guide](https://github.com/augentic/omnia/blob/main/docs/guides/production-backends.md)).

## Usage

Add this crate to your `Cargo.toml` and use it in your runtime configuration:

```rust,ignore
use omnia_wasi_sql::{SqlDefault, WasiSql};

omnia::runtime!({
    hosts: {
        WasiSql: SqlDefault,
    }
});
```

## License

MIT OR Apache-2.0
