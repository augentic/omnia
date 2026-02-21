# Omnia WASI SQL

This crate provides the SQL database interface for the Omnia runtime.

## Interface

Implements the `wasi:sql` WIT interface.

## Backend

- **Host**: Uses `rusqlite` to provide a `SQLite` backend. Supports both in-memory (`:memory:`) and file-based databases.

## Features

### Guest ORM Layer

The guest module provides query builders for type-safe database operations:

- **Entity macro**: Declare database models with automatic trait implementations.
- **Query builders**: Fluent APIs for SELECT, INSERT, UPDATE, DELETE.
- **Joins & Filters**: Type-safe query construction.

## Usage

Add this crate to your `Cargo.toml` and use it in your runtime configuration:

```rust,ignore
use omnia::runtime;
use omnia_wasi_sql::SqlDefault;

omnia::runtime!({
    "sql": SqlDefault,
});
```

## License

MIT OR Apache-2.0
