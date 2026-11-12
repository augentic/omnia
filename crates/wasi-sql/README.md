# Omnia WASI SQL

This crate provides the SQL database interface for the Omnia runtime.

## Interface

Implements the `wasi:sql` WIT interface.

A guest reaches it through `omnia_sdk::SqlDb`. The provider's own handle opens the database `default`, and `named(name)` opens another database the host declares. `wasi:sql` has no transactions, so `SqlDb::batch` fails with `ErrorKind::Unsupported` over it.

## Backend

- **Host**: Uses `rusqlite` to provide a `SQLite` backend. Supports both in-memory (`:memory:`) and file-based databases.

## Configuration

`SqlDefault` holds one `SQLite` database per declared name and refuses to open any other. Names match case-insensitively.

| Variable | Purpose |
| -------- | ------- |
| `SQL_DATABASE` | The URI of the database `default`. Unset, it is one in-memory database shared across the process. |
| `SQL_DATABASES` | The further names a guest may open, comma-separated. |
| `SQL_DATABASE__<NAME>` | The URI of the listed name `<name>`. Unset, the name gets a private in-memory database. |

## Cargo features

- `sqlite` *(default)*: the bundled-SQLite default backend (`SqlDefault`). Disable it (`default-features = false`) when supplying your own `SqlClient` backend to skip compiling bundled `SQLite`.

## Features

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
